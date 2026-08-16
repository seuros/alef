use crate::snippets::error::{Error, Result};
use crate::snippets::types::Language;
use crate::snippets::validators::run_command;
use std::collections::{BTreeMap, HashMap};
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

/// The stable, persistent, cross-run scratch directory for a session's fingerprint, nested under
/// its `working_directory`. Shared between `ValidationSession::workspace_directory` (which
/// creates it) and `purge_stale_workspace_scratch_files` (which needs the identical path before a
/// `ValidationSession` exists to compute it from). ~keep
fn workspace_scratch_directory(working_directory: &Path, fingerprint: &str) -> PathBuf {
    working_directory.join(".alef/snippets/sessions").join(fingerprint)
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

    pub fn temp_dir(&self) -> Result<tempfile::TempDir> {
        let scratch_root = self.working_directory.join(".alef/snippets/tmp");
        std::fs::create_dir_all(&scratch_root)?;
        tempfile::Builder::new()
            .prefix(".alef-snippet-")
            .tempdir_in(scratch_root)
            .map_err(Into::into)
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

pub(crate) fn prepare_sessions_isolated(specs: &HashMap<String, SessionSpec>, timeout_secs: u64) -> SessionPreparation {
    let mut sessions = HashMap::new();
    let mut errors = HashMap::new();
    for (target, spec) in specs {
        match prepare_session(spec, timeout_secs) {
            Ok(session) => {
                sessions.insert(target.clone(), session);
            }
            Err(error) => {
                let message = format!("preparing snippet validation target `{target}`: {error}");
                // Every snippet targeting this session ends up `SnippetStatus::Error` (see
                // `runner::session_preparation_error`) with no other signal that the *target*,
                // not the individual snippets, is what broke — this had zero `tracing::` calls
                // before, so a whole language's worth of results going Error was silent beyond
                // the final summary counts. ~keep
                tracing::error!(
                    target = %target,
                    language = %spec.language,
                    error = %error,
                    "snippet validation session preparation failed"
                );
                errors.insert(target.clone(), message);
            }
        }
    }
    SessionPreparation { sessions, errors }
}

fn prepare_session(spec: &SessionSpec, timeout_secs: u64) -> Result<ValidationSession> {
    let language = spec.language;
    ensure_directory(&spec.working_directory, language)?;
    cleanup_legacy_scratch_directories(&spec.working_directory, timeout_secs)?;
    if let Some(manifest) = &spec.manifest
        && !manifest.is_file()
    {
        return Err(Error::Other(format!(
            "configured {language} snippet manifest does not exist: {}",
            manifest.display()
        )));
    }
    let fingerprint = session_fingerprint(spec)?;
    // `workspace_directory` (csharp, typescript — java moved to `external_workspace_directory`,
    // outside `working_directory`, because alef's own Java backend makes the whole project
    // directory a live Maven source root; see `external_workspace_directory`'s doc comment) is a
    // stable directory reused across every snippet in this session *and* across every future run
    // with an unchanged fingerprint — deliberately, so compiled-artifact caches in its
    // subdirectories survive between runs. But the scratch source file each snippet's validate
    // call writes at its top level (`Program.cs`, `snippet.ts`, ...) is never removed, so it
    // accumulates one leftover file per distinct snippet ever validated under this fingerprint. A
    // consumer-configured `before` command that builds the whole module from `working_directory`
    // runs once for the whole session, before any of *this* run's snippets are written — so it
    // can only ever trip over a leftover from a *previous* run, and one bad leftover then blacks
    // out every snippet in the session. Purging stale top-level files before `before` runs breaks
    // that cycle without touching cache subdirectories (`target/`, `.nuget/`, ...), which is why
    // this only removes direct children that are files, never recursing. ~keep
    purge_stale_workspace_scratch_files(&spec.working_directory, &fingerprint)?;
    for command in &spec.before {
        run_before(command, &spec.working_directory, &spec.env, timeout_secs)
            .map_err(|error| Error::Other(format!("preparing {language} snippet validation session: {error}")))?;
    }
    let session = ValidationSession {
        working_directory: spec.working_directory.clone(),
        manifest: spec.manifest.clone(),
        fingerprint,
        env: spec.env.clone(),
        include_paths: spec.include_paths.clone(),
        rust_features: spec.rust_features.clone(),
        rust_dependencies: spec.rust_dependencies.clone(),
    };
    for directory in [session.cache_directories().0, session.cache_directories().1] {
        std::fs::create_dir_all(&directory).map_err(|error| {
            Error::Other(format!(
                "creating snippet toolchain cache {}: {error}",
                directory.display()
            ))
        })?;
    }
    Ok(session)
}

/// Removes stray top-level files (never directories, never recursing) left in a session's
/// persistent `workspace_directory` by a previous run's per-snippet validate calls. See the
/// `~keep` comment in `prepare_session` for why this must run before `before` hooks. A directory
/// that does not exist yet (the common case: first run for this fingerprint) is not an error. ~keep
fn purge_stale_workspace_scratch_files(working_directory: &Path, fingerprint: &str) -> Result<()> {
    let directory = workspace_scratch_directory(working_directory, fingerprint);
    let entries = match std::fs::read_dir(&directory) {
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
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "neutral-fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let scratch = session.temp_dir().expect("isolated scratch directory");
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
