//! A cache that cannot prevent its own expensive setup step is barely a cache: before this fix,
//! `run_validation` handed every configured session to `prepare_sessions_isolated` before ever
//! checking a `--changed-only` result-cache hit, so a session's `before` hook -- the step that
//! cold-builds a whole native package -- ran unconditionally even on a fully cached run. These
//! tests pin the fix at the one place it is safe to observe without a real toolchain: a session
//! whose only claiming snippet already has a cache hit must not run its `before` hook a second
//! time, and a genuinely uncached snippet must still get one.

use super::*;
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{Language, SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// Always available, never batches, and counts every real toolchain invocation so a test can
/// assert against work that actually happened rather than a status alone.
struct CountingValidator {
    language: Language,
    invocations: Arc<Mutex<usize>>,
}

impl SnippetValidator for CountingValidator {
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
    ) -> Result<(SnippetStatus, Option<String>)> {
        *self.invocations.lock().expect("invocations") += 1;
        Ok((SnippetStatus::Pass, None))
    }

    // The default `validate_in_session` rejects any snippet handed a real session, which every
    // snippet here is (that is the whole point of the fixture). Overriding it to ignore the
    // session and call `validate` directly keeps this validator focused on counting invocations,
    // not on exercising binding-aware session plumbing no other validator here needs.
    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        _session: Option<&crate::snippets::session::ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        self.validate(snippet, level, timeout_secs)
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

fn snippet(language: Language) -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language,
        title: None,
        code: "example snippet body".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

/// Render `path` the way the `sh` a `touch` hook runs under can read it on either platform --
/// mirrors `session::tests::sh_path`, duplicated here because that helper is private to a
/// different module's `#[cfg(test)]` block.
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A `before` hook that creates `marker`, wrapped in `sh -c` on Windows the same way
/// `session::tests::posix_hook` wraps its instruments -- `cmd` cannot parse `touch` at all.
fn touch_hook(marker: &Path) -> String {
    let script = format!("touch {}", sh_path(marker));
    if cfg!(windows) {
        format!("sh -c '{script}'")
    } else {
        script
    }
}

fn python_session(working_directory: &Path, marker: &Path) -> SessionSpec {
    SessionSpec {
        language: Language::Python,
        working_directory: working_directory.to_path_buf(),
        manifest: None,
        before: vec![touch_hook(marker)],
        env: BTreeMap::new(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: BTreeMap::new(),
    }
}

/// The headline fix: a second `--changed-only` pass over the identical snippet, session and
/// cache directory must not touch the `before` hook's marker again, and must not invoke the
/// validator again either -- both signals of "this session's expensive setup ran a second time
/// for zero new work". Reverting the `run_validation` reordering (checking the cache before
/// preparing sessions) makes this fail: the marker reappears and `invocations` becomes 2.
#[test]
fn a_fully_cached_run_does_not_rerun_the_sessions_before_hook() {
    let working_directory = tempfile::tempdir().expect("working directory");
    let cache_directory = tempfile::tempdir().expect("cache directory");
    let marker = working_directory.path().join("before-hook-ran");

    let mut registry = ValidatorRegistry::new();
    let invocations = Arc::new(Mutex::new(0_usize));
    registry.register(Box::new(CountingValidator {
        language: Language::Python,
        invocations: Arc::clone(&invocations),
    }));

    let mut sessions = HashMap::new();
    sessions.insert("python".to_string(), python_session(working_directory.path(), &marker));

    let config = RunnerConfig {
        level: ValidationLevel::Syntax,
        parallelism: 1,
        cache_dir: Some(cache_directory.path().to_path_buf()),
        changed_only: true,
        sessions,
        ..RunnerConfig::default()
    };
    let snippets = vec![snippet(Language::Python)];

    let first = run_validation(&snippets, &registry, &config).expect("first run completes");
    assert_eq!(
        first.passed, 1,
        "the uncached first pass must validate and pass the snippet"
    );
    assert!(marker.exists(), "the before hook must run on the first, uncached pass");
    assert_eq!(*invocations.lock().expect("invocations"), 1);

    std::fs::remove_file(&marker).expect("remove marker between runs");

    let second = run_validation(&snippets, &registry, &config).expect("second run completes");
    assert_eq!(
        second.passed, 1,
        "the cached second pass must still report the same passing result"
    );
    assert!(
        !marker.exists(),
        "a fully cached run must not re-run the session's before hook"
    );
    assert_eq!(
        *invocations.lock().expect("invocations"),
        1,
        "a fully cached run must not invoke the validator a second time either"
    );
}

/// The negative control: a session with a genuinely uncached snippet (a second, never-before-seen
/// snippet added to the mix) must still run its `before` hook, even though this run also carries
/// an already-cached snippet on the same session. A fix that stopped activation unconditionally
/// -- rather than only when every claiming snippet is cached -- would pass the positive test above
/// and silently break this one.
#[test]
fn a_session_with_any_uncached_snippet_still_runs_its_before_hook() {
    let working_directory = tempfile::tempdir().expect("working directory");
    let cache_directory = tempfile::tempdir().expect("cache directory");
    let marker = working_directory.path().join("before-hook-ran");

    let mut registry = ValidatorRegistry::new();
    let invocations = Arc::new(Mutex::new(0_usize));
    registry.register(Box::new(CountingValidator {
        language: Language::Python,
        invocations: Arc::clone(&invocations),
    }));

    let mut sessions = HashMap::new();
    sessions.insert("python".to_string(), python_session(working_directory.path(), &marker));

    let config = RunnerConfig {
        level: ValidationLevel::Syntax,
        parallelism: 1,
        cache_dir: Some(cache_directory.path().to_path_buf()),
        changed_only: true,
        sessions,
        ..RunnerConfig::default()
    };
    let cached_snippet = snippet(Language::Python);

    let first = run_validation(&[cached_snippet.clone()], &registry, &config).expect("first run completes");
    assert_eq!(first.passed, 1);
    std::fs::remove_file(&marker).expect("remove marker between runs");
    assert_eq!(*invocations.lock().expect("invocations"), 1);

    let mut new_snippet = snippet(Language::Python);
    new_snippet.code = "a body never seen before, so it cannot be a cache hit".into();
    let second = run_validation(&[cached_snippet, new_snippet], &registry, &config).expect("second run completes");

    assert_eq!(
        second.passed, 2,
        "both the cached and the uncached snippet must still pass"
    );
    assert!(
        marker.exists(),
        "a session with any uncached snippet must still run its before hook"
    );
    assert_eq!(
        *invocations.lock().expect("invocations"),
        2,
        "only the genuinely uncached snippet should have reached the validator a second time"
    );
}
