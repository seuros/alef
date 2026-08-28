//! The preflight has to be worth exactly two things and no more: it must spawn nothing when the
//! session's artifacts are missing, and it must change nothing when they are present.
//!
//! The second half is not decoration. A "fix" that simply stopped validating would pass every
//! assertion about the first half, and would be indistinguishable from this one by any test that
//! only checks the unsatisfiable case -- which is why every skip assertion here has a satisfiable
//! twin that asserts the validator really ran. Each validator counts its own invocations, so
//! "nothing was spawned" is read off the work itself rather than inferred from a status. ~keep

use super::{RunnerConfig, run_validation};
use crate::snippets::error::Error;
use crate::snippets::session::{SessionSpec, ValidationSession};
use crate::snippets::types::{
    Language, Snippet, SnippetAnnotation, SnippetAnnotationKind, SnippetMetadata, SnippetStatus, SourceOrigin,
    ValidationLevel,
};
use crate::snippets::validators::{BatchValidation, SnippetValidation, SnippetValidator, ValidatorRegistry};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What a probe validator does when it is actually reached.
#[derive(Clone, Copy)]
enum Behaviour {
    Pass,
    Fail,
    TimeOut,
}

/// A validator that records every invocation it receives and declares whatever artifacts the test
/// tells it to. Counting invocations rather than asserting on the resulting statuses is the point:
/// a skip and a pass can both be produced without running anything, and only the counter can tell
/// the intended saving from an accidental one. ~keep
struct ProbeValidator {
    language: Language,
    missing: Vec<PathBuf>,
    behaviour: Behaviour,
    batches: bool,
    invocations: Arc<AtomicUsize>,
}

impl ProbeValidator {
    fn outcome(&self) -> crate::snippets::error::Result<SnippetValidation> {
        match self.behaviour {
            Behaviour::Pass => Ok((SnippetStatus::Pass, None)),
            Behaviour::Fail => Ok((SnippetStatus::Fail, Some("probe rejected the snippet".to_string()))),
            Behaviour::TimeOut => Err(Error::Timeout {
                command: "probe".to_string(),
                timeout_secs: 1,
            }),
        }
    }
}

impl SnippetValidator for ProbeValidator {
    fn language(&self) -> Language {
        self.language
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> crate::snippets::error::Result<SnippetValidation> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        self.outcome()
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        _session: Option<&ValidationSession>,
    ) -> crate::snippets::error::Result<SnippetValidation> {
        self.validate(snippet, level, timeout_secs)
    }

    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        _level: ValidationLevel,
        _timeout_secs: u64,
        _session: Option<&ValidationSession>,
    ) -> Option<crate::snippets::error::Result<BatchValidation>> {
        if !self.batches {
            return None;
        }
        self.invocations.fetch_add(1, Ordering::SeqCst);
        match self.outcome() {
            Ok(value) => Some(Ok(snippets.iter().map(|_| value.clone()).collect())),
            Err(error) => Some(Err(error)),
        }
    }

    fn supports_batching(&self) -> bool {
        self.batches
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn missing_session_artifacts(&self, _session: &ValidationSession, _level: ValidationLevel) -> Vec<PathBuf> {
        self.missing.clone()
    }
}

const TARGET: &str = "zig";

fn snippet(annotation: Option<SnippetAnnotationKind>) -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: Language::Zig,
        title: None,
        code: "const value = 1;".into(),
        start_line: 1,
        block_index: 0,
        annotation: annotation.map(|kind| SnippetAnnotation { kind, reason: None }),
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

fn session_config(working_directory: &std::path::Path) -> HashMap<String, SessionSpec> {
    let mut sessions = HashMap::new();
    sessions.insert(
        TARGET.to_string(),
        SessionSpec {
            language: Language::Zig,
            working_directory: working_directory.to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        },
    );
    sessions
}

struct Probe {
    registry: ValidatorRegistry,
    invocations: Arc<AtomicUsize>,
}

fn probe(missing: Vec<PathBuf>, behaviour: Behaviour, batches: bool) -> Probe {
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(ProbeValidator {
        language: Language::Zig,
        missing,
        behaviour,
        batches,
        invocations: Arc::clone(&invocations),
    }));
    Probe { registry, invocations }
}

fn config(working_directory: &std::path::Path) -> RunnerConfig {
    RunnerConfig {
        level: ValidationLevel::Compile,
        parallelism: 1,
        cache_dir: None,
        sessions: session_config(working_directory),
        ..RunnerConfig::default()
    }
}

/// The defect, at its own scale in miniature: a session whose build artifacts are absent used to
/// spend one real toolchain process per snippet discovering the same single fact. ~keep
#[test]
fn an_unsatisfiable_session_spawns_no_validator_and_reports_every_snippet_skipped() {
    let workspace = tempfile::tempdir().expect("workspace");
    let probe = probe(
        vec![PathBuf::from("target/release/libsample_ffi.dylib")],
        Behaviour::Pass,
        false,
    );
    let snippets = vec![snippet(None), snippet(None), snippet(None)];

    let summary = run_validation(&snippets, &probe.registry, &config(workspace.path())).expect("validation completes");

    assert_eq!(
        probe.invocations.load(Ordering::SeqCst),
        0,
        "an unsatisfiable session must not spawn a single validator invocation"
    );
    assert_eq!(summary.preflight_skipped, 3, "every snippet must be reported skipped");
    assert_eq!(summary.total, 3);
    assert_eq!(summary.passed, 0, "a skip must never be reported as a pass");
    assert_eq!(summary.fully_verified, 0);
    assert!(
        summary.checked_nothing(),
        "a run that validated nothing must still say so"
    );
    let message = summary.results[0].message.clone().unwrap_or_default();
    assert!(
        message.contains("run `alef build`"),
        "the skip must carry the remedy: {message}"
    );
    assert!(
        message.contains("libsample_ffi.dylib"),
        "the skip must name the artifact it could not find: {message}"
    );
}

/// The control without which the test above proves nothing: with the artifact present, the very
/// same corpus, session and validator must run exactly as before. A fix that always skipped would
/// pass every assertion above and fail here. ~keep
#[test]
fn a_satisfiable_session_still_runs_the_validator_for_every_snippet() {
    let workspace = tempfile::tempdir().expect("workspace");
    let probe = probe(Vec::new(), Behaviour::Pass, false);
    let snippets = vec![snippet(None), snippet(None), snippet(None)];

    let summary = run_validation(&snippets, &probe.registry, &config(workspace.path())).expect("validation completes");

    assert_eq!(
        probe.invocations.load(Ordering::SeqCst),
        3,
        "a satisfiable session must validate every snippet"
    );
    assert_eq!(summary.preflight_skipped, 0);
    assert_eq!(summary.passed, 3);
    assert_eq!(summary.fully_verified, 3);
}

/// The batch path needs its own proof: declining to form the group is a different code path from
/// the per-snippet short-circuit, and a batch that still ran would be one process reporting N
/// failures about a corpus nobody checked. ~keep
#[test]
fn an_unsatisfiable_session_never_forms_a_batch_group_either() {
    let workspace = tempfile::tempdir().expect("workspace");
    let probe = probe(vec![PathBuf::from("dist/index.d.ts")], Behaviour::Pass, true);
    let snippets = vec![snippet(None), snippet(None), snippet(None)];

    let summary = run_validation(&snippets, &probe.registry, &config(workspace.path())).expect("validation completes");

    assert_eq!(
        probe.invocations.load(Ordering::SeqCst),
        0,
        "an unsatisfiable session must not spawn a batch invocation either"
    );
    assert_eq!(summary.preflight_skipped, 3);
}

/// The control for the batch path.
#[test]
fn a_satisfiable_session_still_forms_its_batch_group() {
    let workspace = tempfile::tempdir().expect("workspace");
    let probe = probe(Vec::new(), Behaviour::Pass, true);
    let snippets = vec![snippet(None), snippet(None), snippet(None)];

    let summary = run_validation(&snippets, &probe.registry, &config(workspace.path())).expect("validation completes");

    assert_eq!(
        probe.invocations.load(Ordering::SeqCst),
        1,
        "one batch invocation must still cover the whole group"
    );
    assert_eq!(summary.passed, 3);
    assert_eq!(summary.preflight_skipped, 0);
}

/// A missing build artifact bounds `compile`/`typecheck`/`run`, never `syntax`: a syntax check
/// resolves nothing and links nothing, so an author who suppressed a snippet down to
/// `syntax-only` must still get it checked inside an otherwise unsatisfiable session. Skipping it
/// too would be the preflight quietly widening its own remit. ~keep
#[test]
fn a_syntax_only_snippet_is_still_validated_inside_an_unsatisfiable_session() {
    let workspace = tempfile::tempdir().expect("workspace");
    let probe = probe(
        vec![PathBuf::from("target/release/libsample_ffi.dylib")],
        Behaviour::Pass,
        false,
    );
    let snippets = vec![snippet(Some(SnippetAnnotationKind::SyntaxOnly)), snippet(None)];

    let summary = run_validation(&snippets, &probe.registry, &config(workspace.path())).expect("validation completes");

    assert_eq!(
        probe.invocations.load(Ordering::SeqCst),
        1,
        "the syntax-only snippet must still reach the validator"
    );
    assert_eq!(
        summary.preflight_skipped, 1,
        "only the compile-level snippet is skipped"
    );
}

/// A timeout is a stopwatch reading, not a verdict on the snippet. It must stay in the failing
/// counts -- an unbounded toolchain is a real problem -- while being countable apart from a real
/// validator failure, because a consumer reading "411 failed" cannot otherwise tell which of the
/// two they have. ~keep
#[test]
fn a_timeout_is_counted_apart_from_a_validation_failure() {
    let workspace = tempfile::tempdir().expect("workspace");
    let timed_out = probe(Vec::new(), Behaviour::TimeOut, false);
    let snippets = vec![snippet(None), snippet(None)];

    let summary =
        run_validation(&snippets, &timed_out.registry, &config(workspace.path())).expect("validation completes");

    assert_eq!(summary.timed_out, 2, "both invocations ran out of clock");
    assert_eq!(summary.errors, 2, "a timeout still fails the run");
    assert_eq!(summary.failed, 0, "a timeout is not a validator failure");
    assert!(summary.has_failures(), "a timed-out run must not read as clean");
}

/// The negative control for the timeout accounting: a validator that really rejects a snippet must
/// not be counted as a timeout, or the new number would be as uninformative as the old one. ~keep
#[test]
fn a_real_validation_failure_is_never_counted_as_a_timeout() {
    let workspace = tempfile::tempdir().expect("workspace");
    let rejected = probe(Vec::new(), Behaviour::Fail, false);
    let snippets = vec![snippet(None), snippet(None)];

    let summary =
        run_validation(&snippets, &rejected.registry, &config(workspace.path())).expect("validation completes");

    assert_eq!(summary.failed, 2);
    assert_eq!(summary.timed_out, 0, "a rejected snippet is not a timeout");
    assert_eq!(summary.errors, 0);
}
