use crate::snippets::error::{Error, Result};
use crate::snippets::types::Language;
use crate::snippets::validators::run_command;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub language: Language,
    pub working_directory: PathBuf,
    pub manifest: Option<PathBuf>,
    pub before: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub include_paths: Vec<PathBuf>,
    pub rust_features: Vec<String>,
    pub rust_dependencies: BTreeMap<String, crate::core::config::output::DocsSnippetRustDependencyConfig>,
}

#[derive(Debug, Clone)]
pub struct ValidationSession {
    /// Carried from the [`SessionSpec`] so the scratch destination can be one decision rather than
    /// per-runner behaviour: [`crate::snippets::scratch::scratch_root`] needs the language, and a
    /// validator holds only a `ValidationSession`. ~keep
    pub language: Language,
    pub working_directory: PathBuf,
    pub manifest: Option<PathBuf>,
    pub fingerprint: String,
    pub env: BTreeMap<String, String>,
    pub include_paths: Vec<PathBuf>,
    pub rust_features: Vec<String>,
    pub rust_dependencies: BTreeMap<String, crate::core::config::output::DocsSnippetRustDependencyConfig>,
}

pub(crate) struct SessionPreparation {
    pub sessions: HashMap<String, ValidationSession>,
    pub errors: HashMap<String, String>,
}

/// The in-tree root every fingerprint-keyed session scratch directory is nested under, relative to
/// a session's `working_directory`.
const SESSION_SCRATCH_ROOT: &str = ".alef/snippets/sessions";

/// The stable, persistent, cross-run scratch directory for a session's fingerprint, nested under
/// its `working_directory`. Shared between `ValidationSession::workspace_directory` (which
/// creates it) and `purge_session_scratch_root` (which needs the identical path before a
/// `ValidationSession` exists to compute it from). ~keep
fn workspace_scratch_directory(working_directory: &Path, fingerprint: &str) -> PathBuf {
    working_directory.join(SESSION_SCRATCH_ROOT).join(fingerprint)
}

/// Whether a language's per-session scratch lives outside `working_directory` entirely, which is
/// the same question as "does this language have a live in-tree scratch directory at all".
///
/// Java is the only such language: alef's own Java backend points Maven's `<sourceDirectory>` at
/// `${project.basedir}`, so `JavaValidator` resolves its scratch through
/// [`ValidationSession::external_workspace_directory`] instead. That pairing is held in place by
/// `JavaValidator`'s `session_scratch_is_never_written_under_the_working_directory` regression
/// test, which fails if the validator ever writes under `working_directory` again.
const fn keeps_scratch_outside_working_directory(language: Language) -> bool {
    matches!(language, Language::Java)
}

impl ValidationSession {
    pub fn workspace_directory(&self) -> Result<PathBuf> {
        let directory = workspace_scratch_directory(&self.working_directory, &self.fingerprint);
        std::fs::create_dir_all(&directory)?;
        Ok(directory)
    }

    /// The persistent, fingerprint-keyed scratch directory for a session whose build tool globs
    /// its whole project directory for sources, not just a `src/` subtree — alef's own Java
    /// backend sets Maven's `<sourceDirectory>` to `${project.basedir}` (see the generated
    /// `packages/java/pom.xml`) because it emits sources at the package root rather than under
    /// `src/main/java/`. That means every path under a Java session's `working_directory`,
    /// `.alef/` included, is a live compiler input: `mvn package` would compile scratch
    /// `.java` files into the shipped artifact, and `maven-source-plugin`/`javadoc` would bundle
    /// them too. This directory lives under the OS temp root instead, so it can never be
    /// swept up by the consumer's own build. Classpath resolution is unaffected because
    /// `JavaValidator` resolves classpath entries as absolute paths from the manifest,
    /// independent of where the scratch source and class files are compiled from. ~keep
    pub fn external_workspace_directory(&self) -> Result<PathBuf> {
        let directory = std::env::temp_dir()
            .join("alef-snippets/sessions")
            .join(&self.fingerprint);
        std::fs::create_dir_all(&directory)?;
        Ok(directory)
    }

    /// Where this session's per-snippet scratch is allocated. Delegates to
    /// [`crate::snippets::scratch::scratch_root`] so a runner and the preparation-time sweep can
    /// never disagree about the destination. ~keep
    #[must_use]
    pub fn scratch_root(&self) -> PathBuf {
        crate::snippets::scratch::scratch_root(self.language, &self.working_directory, self.manifest.as_deref())
    }

    /// Allocates a self-removing scratch directory for this session.
    ///
    /// # Errors
    ///
    /// Returns an error when the scratch root cannot be created or a unique directory cannot be
    /// allocated inside it.
    pub fn scratch_dir(&self) -> Result<crate::snippets::scratch::ScratchDir> {
        crate::snippets::scratch::ScratchDir::for_session(self)
    }

    pub fn apply(&self, command: &mut std::process::Command) {
        command.current_dir(&self.working_directory);
        self.apply_environment(command);
    }

    pub fn apply_environment(&self, command: &mut std::process::Command) {
        let (go_cache, zig_cache) = self.cache_directories();
        command.env("GOCACHE", &go_cache);
        command.env("ZIG_GLOBAL_CACHE_DIR", &zig_cache);
        for (name, value) in &self.env {
            let path = std::path::Path::new(value);
            let value = if matches!(name.as_str(), "GOCACHE" | "ZIG_GLOBAL_CACHE_DIR") && path.is_relative() {
                self.working_directory.join(path).into_os_string()
            } else {
                value.into()
            };
            command.env(name, value);
        }
    }

    fn cache_directories(&self) -> (PathBuf, PathBuf) {
        let root = self
            .working_directory
            .join(".alef/snippets/cache")
            .join(&self.fingerprint);
        (root.join("go-build"), root.join("zig-global"))
    }
}

/// Prepares every configured session in three phases, because the middle one is not per-session.
///
/// Phase one resolves each spec far enough to know its fingerprint, without running any `before`
/// hook. Phase two then purges the in-tree session scratch root of every working directory, using
/// the *complete* set of live fingerprints for that directory — which is why it cannot be folded
/// back into a per-session step: two targets can legitimately share one `working_directory`, and a
/// per-session purge would delete its sibling's live scratch. Phase three runs the `before` hooks
/// against the already-purged tree.
pub(crate) fn prepare_sessions_isolated(specs: &HashMap<String, SessionSpec>, timeout_secs: u64) -> SessionPreparation {
    let mut sessions = HashMap::new();
    let mut errors = HashMap::new();
    let mut resolved = Vec::new();
    for (target, spec) in specs {
        match resolve_session(spec, timeout_secs) {
            Ok(session) => resolved.push((target, spec, session)),
            Err(error) => record_preparation_error(&mut errors, target, spec, &error),
        }
    }
    purge_stale_session_scratch(&resolved);
    for (target, spec, session) in resolved {
        match activate_session(spec, &session, timeout_secs) {
            Ok(()) => {
                sessions.insert(target.clone(), session);
            }
            Err(error) => record_preparation_error(&mut errors, target, spec, &error),
        }
    }
    SessionPreparation { sessions, errors }
}

fn record_preparation_error(errors: &mut HashMap<String, String>, target: &str, spec: &SessionSpec, error: &Error) {
    let message = format!("preparing snippet validation target `{target}`: {error}");
    // Every snippet targeting this session ends up `SnippetStatus::Error` (see
    // `runner::session_preparation_error`) with no other signal that the *target*, not the
    // individual snippets, is what broke — this had zero `tracing::` calls before, so a whole
    // language's worth of results going Error was silent beyond the final summary counts. ~keep
    tracing::error!(
        target = %target,
        language = %spec.language,
        error = %error,
        "snippet validation session preparation failed"
    );
    errors.insert(target.to_owned(), message);
}

/// Validates a spec and derives its fingerprint. Deliberately runs no `before` hook: the hook must
/// not see a scratch root that `purge_stale_session_scratch` has not swept yet, and that sweep
/// needs every session's fingerprint first.
fn resolve_session(spec: &SessionSpec, timeout_secs: u64) -> Result<ValidationSession> {
    let language = spec.language;
    ensure_directory(&spec.working_directory, language)?;
    cleanup_legacy_scratch_directories(&spec.working_directory, timeout_secs)?;
    purge_abandoned_scratch(spec, timeout_secs);
    if let Some(manifest) = &spec.manifest
        && !manifest.is_file()
    {
        return Err(Error::Other(format!(
            "configured {language} snippet manifest does not exist: {}",
            manifest.display()
        )));
    }
    Ok(ValidationSession {
        language,
        working_directory: spec.working_directory.clone(),
        manifest: spec.manifest.clone(),
        fingerprint: session_fingerprint(spec)?,
        env: spec.env.clone(),
        include_paths: spec.include_paths.clone(),
        rust_features: spec.rust_features.clone(),
        rust_dependencies: spec.rust_dependencies.clone(),
    })
}

fn activate_session(spec: &SessionSpec, session: &ValidationSession, timeout_secs: u64) -> Result<()> {
    let language = spec.language;
    for command in &spec.before {
        run_before(command, &spec.working_directory, &spec.env, timeout_secs)
            .map_err(|error| Error::Other(format!("preparing {language} snippet validation session: {error}")))?;
    }
    for directory in [session.cache_directories().0, session.cache_directories().1] {
        std::fs::create_dir_all(&directory).map_err(|error| {
            Error::Other(format!(
                "creating snippet toolchain cache {}: {error}",
                directory.display()
            ))
        })?;
    }
    Ok(())
}

/// Sweeps the in-tree session scratch root of every working directory this run touches, keeping
/// only the fingerprints this run is actually about to use.
///
/// Two distinct leftovers accumulate there and both black out a whole language:
///
/// - A *stale fingerprint's whole directory*. The fingerprint changes whenever the working tree
///   does, so every day's run mints a new directory and nothing ever removed the previous ones. In
///   a Java package this is fatal rather than untidy: alef's Java backend points Maven's
///   `<sourceDirectory>` at `${project.basedir}`, so a `before` hook of `mvn package` compiles
///   every `.java` under it — four days' worth of leftover `Example.java` at once, which `javac`
///   rejects with `duplicate class: Example`, failing session preparation and stamping every
///   snippet of that language `SnippetStatus::Error`. `JavaValidator` has written its scratch
///   outside `working_directory` since the `external_workspace_directory` fix, so java's live set
///   here is empty and every directory it finds is a pre-fix leftover — but the accumulation is
///   not java-specific, so neither is the sweep.
/// - A *stray top-level file inside a live fingerprint's directory*, left by a previous run's
///   per-snippet validate call (`Program.cs`, `snippet.ts`, ...). That directory is deliberately
///   reused across runs so compiled-artifact caches in its subdirectories survive, so it is kept
///   and only its direct file children are removed — never a subdirectory (`target/`, `.nuget/`,
///   `dist/`, ...), never recursing.
///
/// A sweep failure is logged and tolerated rather than propagated: losing the sweep costs
/// cleanliness, while failing preparation over it would black out exactly the language the sweep
/// exists to keep running. ~keep
fn purge_stale_session_scratch(resolved: &[(&String, &SessionSpec, ValidationSession)]) {
    let mut live: BTreeMap<&Path, BTreeSet<&str>> = BTreeMap::new();
    for (_, spec, session) in resolved {
        let fingerprints = live.entry(spec.working_directory.as_path()).or_default();
        if !keeps_scratch_outside_working_directory(spec.language) {
            fingerprints.insert(session.fingerprint.as_str());
        }
    }
    for (working_directory, fingerprints) in live {
        if let Err(error) = purge_session_scratch_root(working_directory, &fingerprints) {
            tracing::warn!(
                working_directory = %working_directory.display(),
                error = %error,
                "could not purge stale snippet session scratch"
            );
        }
    }
}

fn purge_session_scratch_root(working_directory: &Path, live_fingerprints: &BTreeSet<&str>) -> Result<()> {
    let root = working_directory.join(SESSION_SCRATCH_ROOT);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::Other(format!(
                "reading snippet session scratch root {}: {error}",
                root.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            Error::Other(format!(
                "reading an entry in snippet session scratch root {}: {error}",
                root.display()
            ))
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        let is_live = name.to_str().is_some_and(|name| live_fingerprints.contains(name));
        if !is_directory {
            remove_scratch(std::fs::remove_file(&path), &path)?;
        } else if is_live {
            purge_stale_workspace_scratch_files(&path)?;
        } else {
            remove_scratch(std::fs::remove_dir_all(&path), &path)?;
        }
    }
    Ok(())
}

fn remove_scratch(outcome: std::io::Result<()>, path: &Path) -> Result<()> {
    match outcome {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Other(format!(
            "removing stale snippet session scratch {}: {error}",
            path.display()
        ))),
    }
}

/// Removes stray top-level files (never directories, never recursing) left in a live session
/// scratch `directory` by a previous run's per-snippet validate calls. See
/// [`purge_stale_session_scratch`] for why this must run before `before` hooks. A directory that
/// does not exist yet (the common case: first run for this fingerprint) is not an error.
fn purge_stale_workspace_scratch_files(directory: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::Other(format!(
                "reading snippet workspace directory {}: {error}",
                directory.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            Error::Other(format!(
                "reading an entry in snippet workspace directory {}: {error}",
                directory.display()
            ))
        })?;
        let is_stale_file = entry.file_type().is_ok_and(|file_type| file_type.is_file());
        if !is_stale_file {
            continue;
        }
        if let Err(error) = std::fs::remove_file(entry.path())
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(Error::Other(format!(
                "removing stale snippet scratch file {}: {error}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// Sweeps the scratch root [`crate::snippets::scratch::scratch_root`] chose for this spec, removing
/// entries abandoned by a run that was killed before its guards could drop.
///
/// This exists because moving scratch under `.alef/snippets/tmp` pointed
/// [`cleanup_legacy_scratch_directories`] — which only ever reads the top level of
/// `working_directory` — at a set that no longer contains any scratch at all. Without this the
/// only remaining cleanup was in-process `Drop`, which a `SIGINT` skips entirely, so leftovers
/// accumulated in the cache root indefinitely.
///
/// Logged and tolerated rather than propagated, for the same reason as
/// [`purge_stale_session_scratch`]: losing a sweep costs cleanliness, while failing preparation
/// over it would black out exactly the language the sweep exists to keep clean. ~keep
fn purge_abandoned_scratch(spec: &SessionSpec, timeout_secs: u64) {
    let root = crate::snippets::scratch::scratch_root(spec.language, &spec.working_directory, spec.manifest.as_deref());
    if let Err(error) = crate::snippets::scratch::purge_stale_scratch_root(&root, timeout_secs) {
        tracing::warn!(
            scratch_root = %root.display(),
            language = %spec.language,
            error = %error,
            "could not purge abandoned snippet scratch"
        );
    }
}

/// Sweeps `.alef-snippet-*` directories left *directly* in `working_directory` by alef versions
/// that predate the single scratch destination. Nothing writes there any more, so this covers only
/// pre-fix leftovers; abandoned scratch from the current layout is [`purge_abandoned_scratch`]'s
/// job. Deliberately keyed on the `.alef-snippet-` prefix and on directories only: this root is
/// the consumer's own source directory, not alef's, so anything less specific would be a delete
/// gate pointed at tracked files. ~keep
fn cleanup_legacy_scratch_directories(working_directory: &Path, timeout_secs: u64) -> Result<()> {
    let stale_after = std::time::Duration::from_secs(timeout_secs.saturating_add(60));
    let entries = std::fs::read_dir(working_directory).map_err(|error| {
        Error::Other(format!(
            "reading snippet working directory {}: {error}",
            working_directory.display()
        ))
    })?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error::Other(format!(
                    "reading an entry in snippet working directory {}: {error}",
                    working_directory.display()
                )));
            }
        };
        let entry_type = match entry.file_type() {
            Ok(entry_type) => entry_type,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error::Other(format!(
                    "reading snippet scratch entry type {}: {error}",
                    entry.path().display()
                )));
            }
        };
        if !entry_type.is_dir() || !entry.file_name().to_string_lossy().starts_with(".alef-snippet-") {
            continue;
        }
        let modified = match entry.metadata() {
            Ok(metadata) => metadata.modified().map_err(|error| {
                Error::Other(format!(
                    "reading snippet scratch modification time {}: {error}",
                    entry.path().display()
                ))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error::Other(format!(
                    "reading snippet scratch metadata {}: {error}",
                    entry.path().display()
                )));
            }
        };
        if modified.elapsed().is_ok_and(|age| age >= stale_after)
            && let Err(error) = std::fs::remove_dir_all(entry.path())
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(Error::Other(format!(
                "removing stale snippet scratch directory {}: {error}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

fn session_fingerprint(spec: &SessionSpec) -> Result<String> {
    const IGNORED_DIRECTORIES: &[&str] = &[".git", ".alef", "target", "node_modules", ".venv", "build"];
    let mut paths = walkdir::WalkDir::new(&spec.working_directory)
        .into_iter()
        .filter_entry(|entry| {
            !entry.file_type().is_dir() || !IGNORED_DIRECTORIES.contains(&entry.file_name().to_string_lossy().as_ref())
        })
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    paths.sort();
    let mut hasher = blake3::Hasher::new();
    hasher.update(spec.working_directory.to_string_lossy().as_bytes());
    if let Some(manifest) = &spec.manifest {
        hasher.update(manifest.to_string_lossy().as_bytes());
    }
    for command in &spec.before {
        hasher.update(command.as_bytes());
    }
    for (name, value) in &spec.env {
        hasher.update(name.as_bytes());
        hasher.update(value.as_bytes());
    }
    for path in &spec.include_paths {
        hasher.update(path.to_string_lossy().as_bytes());
    }
    for feature in &spec.rust_features {
        hasher.update(feature.as_bytes());
    }
    for (name, dependency) in &spec.rust_dependencies {
        hasher.update(name.as_bytes());
        hasher.update(dependency.version.as_bytes());
        hasher.update(&[u8::from(dependency.default_features)]);
        for feature in &dependency.features {
            hasher.update(feature.as_bytes());
        }
    }
    for path in paths {
        hasher.update(
            path.strip_prefix(&spec.working_directory)
                .unwrap_or(&path)
                .to_string_lossy()
                .as_bytes(),
        );
        hasher.update(&std::fs::read(&path)?);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn ensure_directory(path: &Path, language: Language) -> Result<()> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "configured {language} snippet working directory does not exist: {}",
            path.display()
        )))
    }
}

fn run_before(source: &str, working_directory: &Path, env: &BTreeMap<String, String>, timeout_secs: u64) -> Result<()> {
    let mut command = shell_command(source);
    command.current_dir(working_directory);
    command.envs(env);
    let (success, output) = run_command(&mut command, timeout_secs)?;
    if success {
        Ok(())
    } else {
        Err(Error::Other(format!("before command failed: {output}")))
    }
}

#[cfg(unix)]
fn shell_command(source: &str) -> std::process::Command {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", source]);
    command
}

#[cfg(windows)]
fn shell_command(source: &str) -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    command.args(["/C", source]);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_before_command_once_per_language() {
        let directory = tempfile::tempdir().expect("temp directory");
        let marker = directory.path().join("prepared");
        let mut specs = HashMap::new();
        specs.insert(
            "python".into(),
            SessionSpec {
                language: Language::Python,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec![format!("test ! -e prepared && touch {}", marker.display())],
                env: BTreeMap::new(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(marker.exists());
        assert!(prepared.errors.is_empty());
        assert_eq!(prepared.sessions.len(), 1);
    }

    #[test]
    fn scratch_cleanup_errors_name_the_working_directory() {
        let directory = tempfile::tempdir().expect("temp directory");
        let missing = directory.path().join("removed");

        let error = cleanup_legacy_scratch_directories(&missing, 5).expect_err("missing root must fail");

        let message = error.to_string();
        assert!(message.contains("reading snippet working directory"));
        assert!(message.contains(&missing.display().to_string()));
    }

    /// A failed target must be visible beyond the final summary counts: every snippet aimed at it
    /// silently becomes `SnippetStatus::Error` downstream (see
    /// `runner::session_preparation_error`), and before this there was no `tracing::` call
    /// anywhere in this module to explain why. ~keep
    #[tracing_test::traced_test]
    #[test]
    fn rejects_missing_configured_manifest() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut specs = HashMap::new();
        specs.insert(
            "typescript".into(),
            SessionSpec {
                language: Language::TypeScript,
                working_directory: directory.path().to_path_buf(),
                manifest: Some(directory.path().join("missing.json")),
                before: Vec::new(),
                env: BTreeMap::new(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);
        let error = prepared.errors.get("typescript").expect("missing manifest is rejected");
        assert!(logs_contain("snippet validation session preparation failed"));
        assert!(logs_contain("typescript"));

        assert!(error.contains("manifest does not exist"));
    }

    /// The regression this closes: a `before` hook that builds the whole module from
    /// `working_directory` (`npm run build`, for a TypeScript session — java no longer takes this
    /// path at all; see `external_workspace_directory`) runs once, before any of *this* run's
    /// snippets are written — so the only way it can trip over bad scratch source content is a
    /// leftover from a *previous* run's per-snippet validate call, which nothing ever cleaned up.
    /// One bad leftover then failed session preparation and stamped every snippet in the session
    /// as `SnippetStatus::Error`, turning one bad snippet into a whole language going dark. The
    /// `before` command below does not know the fingerprint-derived workspace path in advance
    /// (neither does a real consumer's `npm run build`), so it searches for the leftover instead
    /// of asserting a literal path — exactly what a stale-content bug would trip over. ~keep
    #[test]
    fn stale_workspace_scratch_files_are_purged_before_before_hooks_run() {
        let directory = tempfile::tempdir().expect("temp directory");
        let spec = SessionSpec {
            language: Language::TypeScript,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: vec!["! find .alef/snippets/sessions -name snippet.ts | grep -q .".into()],
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let fingerprint = session_fingerprint(&spec).expect("fingerprint");
        let workspace = workspace_scratch_directory(directory.path(), &fingerprint);
        std::fs::create_dir_all(&workspace).expect("workspace directory");
        let stale_file = workspace.join("snippet.ts");
        std::fs::write(&stale_file, "this does not compile: :::").expect("stale scratch file");
        // A subdirectory must survive the purge: it stands in for a compiled-artifact cache
        // (`target/classes`, `.nuget/packages`, ...) that is deliberately reused across runs. ~keep
        let cache_subdir = workspace.join("dist");
        std::fs::create_dir_all(&cache_subdir).expect("cache subdirectory");
        std::fs::write(cache_subdir.join("snippet.js"), b"cached").expect("cached artifact");

        let mut specs = HashMap::new();
        specs.insert("typescript".to_string(), spec);
        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(
            prepared.errors.is_empty(),
            "the `before` hook must run against an already-purged workspace: {:?}",
            prepared.errors
        );
        assert!(!stale_file.exists(), "the stale scratch file must be purged");
        assert!(
            cache_subdir.join("snippet.js").exists(),
            "cache subdirectories must survive the purge"
        );
    }

    /// The java incident this closes: `packages/java/.alef/snippets/sessions/` had accumulated
    /// four fingerprint-keyed directories dated across three separate days. alef's Java backend
    /// points Maven's `<sourceDirectory>` at `${project.basedir}`, so the session's own
    /// `mvn package` `before` hook compiled all four leftovers together and `javac` rejected them
    /// with `duplicate class: Example` — session preparation failed and all 283 java snippets were
    /// skipped. `JavaValidator` has written its scratch outside `working_directory` since the
    /// `external_workspace_directory` fix, so those directories were pre-fix leftovers that
    /// nothing swept: `--clean` only bypasses caches, and the per-fingerprint purge only ever
    /// looked inside the *current* fingerprint's directory. The `before` hook below globs the way
    /// Maven does rather than asserting a literal path, because a real consumer's hook does not
    /// know the fingerprint either. ~keep
    #[test]
    fn a_stale_session_directory_from_a_previous_run_cannot_break_the_current_one() {
        let directory = tempfile::tempdir().expect("temp directory");
        let stale = workspace_scratch_directory(directory.path(), "fingerprint-from-a-previous-run");
        std::fs::create_dir_all(&stale).expect("stale session directory");
        std::fs::write(stale.join("Example.java"), "public class Example {}").expect("stale source");
        std::fs::write(stale.join("Example.class"), b"stale").expect("stale class file");

        let mut specs = HashMap::new();
        specs.insert(
            "java".to_string(),
            SessionSpec {
                language: Language::Java,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec!["! find . -name 'Example.java' | grep -q .".into()],
                env: BTreeMap::new(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(
            prepared.errors.is_empty(),
            "a previous run's leftovers must not reach this run's `before` hook: {:?}",
            prepared.errors
        );
        assert!(
            !stale.exists(),
            "a stale session directory must not survive into the next run"
        );
    }

    /// A stale fingerprint's directory is removed outright while the live fingerprint's is kept
    /// and only swept of stray top-level files — the compiled-artifact caches in its
    /// subdirectories are deliberately reused across runs and must survive. ~keep
    #[test]
    fn a_stale_fingerprint_is_removed_while_the_live_one_keeps_its_caches() {
        let directory = tempfile::tempdir().expect("temp directory");
        let spec = SessionSpec {
            language: Language::TypeScript,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let live = workspace_scratch_directory(directory.path(), &session_fingerprint(&spec).expect("fingerprint"));
        std::fs::create_dir_all(live.join("dist")).expect("live cache directory");
        std::fs::write(live.join("dist/cached.js"), b"cached").expect("cached artifact");
        std::fs::write(live.join("snippet.ts"), "this does not compile: :::").expect("stale scratch file");
        let stale = workspace_scratch_directory(directory.path(), "fingerprint-from-a-previous-run");
        std::fs::create_dir_all(&stale).expect("stale session directory");
        std::fs::write(stale.join("snippet.ts"), "this does not compile: :::").expect("stale scratch file");

        let mut specs = HashMap::new();
        specs.insert("typescript".to_string(), spec);
        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(prepared.errors.is_empty(), "{:?}", prepared.errors);
        assert!(!stale.exists(), "a stale fingerprint's directory must be removed");
        assert!(
            live.join("dist/cached.js").exists(),
            "the live fingerprint's caches must survive"
        );
        assert!(
            !live.join("snippet.ts").exists(),
            "the live fingerprint's stray scratch files must still be swept"
        );
    }

    /// Two targets can legitimately share one `working_directory` while differing in a way that
    /// changes the fingerprint. The purge therefore has to be computed over *all* of a directory's
    /// live fingerprints at once: a per-session purge would let whichever target ran second delete
    /// the first one's live scratch, turning the stale-session fix into a fresh collision. ~keep
    #[test]
    fn sibling_sessions_sharing_a_working_directory_keep_each_others_scratch() {
        let directory = tempfile::tempdir().expect("temp directory");
        let base = SessionSpec {
            language: Language::TypeScript,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let mut node = base.clone();
        node.env = BTreeMap::from([("ALEF_SESSION".into(), "node".into())]);
        let mut wasm = base;
        wasm.env = BTreeMap::from([("ALEF_SESSION".into(), "wasm".into())]);
        let fingerprints = [
            session_fingerprint(&node).expect("node fingerprint"),
            session_fingerprint(&wasm).expect("wasm fingerprint"),
        ];
        assert_ne!(fingerprints[0], fingerprints[1]);
        for fingerprint in &fingerprints {
            let workspace = workspace_scratch_directory(directory.path(), fingerprint);
            std::fs::create_dir_all(workspace.join("dist")).expect("cache directory");
            std::fs::write(workspace.join("dist/cached.js"), b"cached").expect("cached artifact");
        }

        let specs = HashMap::from([("node".to_string(), node), ("wasm".to_string(), wasm)]);
        let prepared = prepare_sessions_isolated(&specs, 5);

        assert!(prepared.errors.is_empty(), "{:?}", prepared.errors);
        for fingerprint in &fingerprints {
            let cached = workspace_scratch_directory(directory.path(), fingerprint).join("dist/cached.js");
            assert!(
                cached.exists(),
                "a sibling session's live scratch must survive: {}",
                cached.display()
            );
        }
    }

    #[test]
    fn applies_environment_to_setup_and_validation_commands() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut specs = HashMap::new();
        specs.insert(
            "zig".into(),
            SessionSpec {
                language: Language::Zig,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec!["test \"$ALEF_SESSION_CACHE\" = configured".into()],
                env: BTreeMap::from([("ALEF_SESSION_CACHE".into(), "configured".into())]),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: BTreeMap::new(),
            },
        );

        let prepared = prepare_sessions_isolated(&specs, 5);
        assert!(prepared.errors.is_empty());
        let session = prepared.sessions.get("zig").expect("zig session");
        let mut command = std::process::Command::new("true");
        session.apply(&mut command);

        assert_eq!(
            command.get_envs().next(),
            Some(("ALEF_SESSION_CACHE".as_ref(), Some("configured".as_ref())))
        );
    }

    #[test]
    fn include_paths_contribute_to_the_session_fingerprint() {
        let directory = tempfile::tempdir().expect("temp directory");
        let base = SessionSpec {
            language: Language::C,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: vec![directory.path().join("include")],
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let mut changed = base.clone();
        changed.include_paths = vec![directory.path().join("vendor/include")];

        assert_ne!(
            session_fingerprint(&base).expect("base fingerprint"),
            session_fingerprint(&changed).expect("changed fingerprint")
        );
    }

    #[test]
    fn reuses_a_stable_workspace_for_a_prepared_session() {
        let directory = tempfile::tempdir().expect("temp directory");
        let session = ValidationSession {
            language: Language::Python,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "neutral-fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let first = session.workspace_directory().expect("first workspace");
        std::fs::write(first.join("compiler-output"), "cached").expect("compiler output");
        let second = session.workspace_directory().expect("second workspace");

        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_to_string(second.join("compiler-output")).unwrap(),
            "cached"
        );
    }

    /// `external_workspace_directory` exists because alef's own Java backend emits sources at
    /// the package root and points Maven's `<sourceDirectory>` at `${project.basedir}` (see
    /// `packages/java/pom.xml`), making every path under a session's `working_directory` a live
    /// compiler input. Unlike `workspace_directory`, it must never resolve under
    /// `working_directory` at all, while still being stable and reused across calls for the same
    /// fingerprint so compiled-artifact caching still works.
    #[test]
    fn external_workspace_directory_stays_outside_the_working_directory_and_is_stable() {
        let directory = tempfile::tempdir().expect("temp directory");
        let fingerprint = format!(
            "external-workspace-fixture-{}",
            directory.path().to_string_lossy().replace(['/', '\\', ':'], "_")
        );
        let session = ValidationSession {
            language: Language::Python,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint,
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let first = session
            .external_workspace_directory()
            .expect("first external workspace");
        assert!(
            !first.starts_with(directory.path()),
            "external workspace must never be nested under working_directory: {}",
            first.display()
        );
        std::fs::write(first.join("compiler-output"), "cached").expect("compiler output");
        let second = session
            .external_workspace_directory()
            .expect("second external workspace");

        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_to_string(second.join("compiler-output")).unwrap(),
            "cached"
        );
        let _ = std::fs::remove_dir_all(&first);
    }

    #[test]
    fn provides_absolute_isolated_toolchain_directories() {
        let directory = tempfile::tempdir().expect("temp directory");
        let session = ValidationSession {
            language: Language::Python,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "neutral-fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let scratch = session.scratch_dir().expect("isolated scratch directory");
        assert!(scratch.path().starts_with(directory.path().join(".alef/snippets/tmp")));
        let mut command = std::process::Command::new("true");
        session.apply_environment(&mut command);
        let values = command
            .get_envs()
            .filter_map(|(name, value)| value.map(|value| (name.to_string_lossy().into_owned(), value.to_owned())))
            .collect::<BTreeMap<_, _>>();

        for name in ["GOCACHE", "ZIG_GLOBAL_CACHE_DIR"] {
            assert!(std::path::Path::new(&values[name]).is_absolute(), "{name}");
        }
    }
}
