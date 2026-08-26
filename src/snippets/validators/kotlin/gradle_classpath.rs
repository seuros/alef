use crate::snippets::error::{Error, Result};
use crate::snippets::session::ValidationSession;
use crate::snippets::validators::run_command;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use super::KotlinValidator;

/// Gradle init-script content that resolves a project's real Kotlin compile classpath and its own
/// compiled-class output directories, printed one per line as `ALEF_CLASSPATH_ENTRY:<path>`.
///
/// Directory probing cannot cover this: AGP's compiled-output layout is variant- and
/// version-dependent (AGP 9.x lands classes at
/// `build/intermediates/built_in_kotlinc/<variant>/compile<Variant>Kotlin/classes`; older AGP used
/// `build/tmp/kotlin-classes/<variant>`), and probing cannot see a project's *dependency*
/// classpath — kotlinx-coroutines, Jackson, etc. — at all. Asking Gradle's own task model instead
/// of guessing a path handles both, and needs no change to the consumer's build file. ~keep
const INIT_SCRIPT: &str = include_str!("assets/alef_classpath_init.gradle");

const INIT_SCRIPT_FILE_NAME: &str = "alef_classpath_init.gradle";
const CLASS_PATH_TASK: &str = "alefPrintClasspath";
const CLASS_PATH_ENTRY_PREFIX: &str = "ALEF_CLASSPATH_ENTRY:";
const MANIFEST_NAMES: [&str; 2] = ["build.gradle.kts", "build.gradle"];

#[cfg(unix)]
const WRAPPER_NAME: &str = "gradlew";
#[cfg(windows)]
const WRAPPER_NAME: &str = "gradlew.bat";

type ClassPathCache = Mutex<HashMap<PathBuf, std::result::Result<OsString, String>>>;

/// Process-wide, keyed by the manifest's canonicalized path. A Gradle invocation costs whole
/// seconds even with a warm daemon, and batch validation resolves a session's classpath once per
/// batch — dozens of times in one `alef snippets check` run — so the first resolution is reused
/// for the rest of the process. ~keep
static CACHE: OnceLock<ClassPathCache> = OnceLock::new();

/// Resolves a Kotlin session's classpath: Gradle's own configuration model for a Gradle project
/// with a wrapper, directory probing (`KotlinValidator::class_path`) otherwise. A Gradle
/// invocation that fails falls back to probing too rather than failing the whole session —
/// probing can still find the project's own compiled output even when dependency resolution
/// cannot run (offline, misconfigured repository, ...); the failure is logged, not swallowed. ~keep
pub(super) fn resolve_class_path(manifest: &Path, session: &ValidationSession, timeout_secs: u64) -> Result<OsString> {
    if is_gradle_manifest(manifest) {
        match gradle_class_path(manifest, session, timeout_secs) {
            Some(Ok(class_path)) => return Ok(class_path),
            Some(Err(error)) => {
                tracing::warn!(
                    manifest = %manifest.display(),
                    error = %error,
                    "Gradle classpath resolution failed; falling back to directory probing"
                );
            }
            None => {}
        }
    }
    KotlinValidator::class_path(manifest)
}

fn is_gradle_manifest(manifest: &Path) -> bool {
    manifest
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| MANIFEST_NAMES.contains(&name))
}

fn gradle_wrapper(root: &Path) -> Option<PathBuf> {
    let wrapper = root.join(WRAPPER_NAME);
    wrapper.is_file().then_some(wrapper)
}

/// `None` when the project has no wrapper to invoke, so the caller falls straight through to
/// directory probing without a wasted attempt.
fn gradle_class_path(manifest: &Path, session: &ValidationSession, timeout_secs: u64) -> Option<Result<OsString>> {
    let root = manifest.parent().unwrap_or_else(|| Path::new("."));
    let wrapper = gradle_wrapper(root)?;
    let key = std::fs::canonicalize(manifest).unwrap_or_else(|_| manifest.to_path_buf());
    let cache = CACHE.get_or_init(ClassPathCache::default);
    if let Some(cached) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return Some(cached.clone().map_err(Error::Other));
    }
    let resolved = run_gradle_class_path(&wrapper, root, session, timeout_secs);
    let stored = match &resolved {
        Ok(class_path) => Ok(class_path.clone()),
        Err(error) => Err(error.to_string()),
    };
    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(key, stored);
    Some(resolved)
}

fn run_gradle_class_path(
    wrapper: &Path,
    root: &Path,
    session: &ValidationSession,
    timeout_secs: u64,
) -> Result<OsString> {
    let script = session.workspace_directory()?.join(INIT_SCRIPT_FILE_NAME);
    std::fs::write(&script, INIT_SCRIPT)?;
    let mut command = std::process::Command::new(wrapper);
    command
        .current_dir(root)
        .args(["--init-script", &script.to_string_lossy(), CLASS_PATH_TASK, "-q"]);
    let (success, output) = run_command(&mut command, timeout_secs)?;
    let entries: Vec<PathBuf> = output
        .lines()
        .filter_map(|line| line.strip_prefix(CLASS_PATH_ENTRY_PREFIX))
        .map(PathBuf::from)
        .collect();
    if !success || entries.is_empty() {
        // `entries` is parsed from the full output above; only the reported prose is bounded. ~keep
        return Err(Error::Other(format!(
            "resolving Gradle classpath for {}: {}",
            root.display(),
            crate::snippets::diagnostics::bounded_text(&output)
        )));
    }
    std::env::join_paths(entries)
        .map_err(|error| Error::Other(format!("building Gradle classpath for {}: {error}", root.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::Language;
    use std::collections::BTreeMap;

    /// The fixtures below are trivial shell scripts that echo a fixed line and exit, so a real
    /// resolution takes milliseconds. The bound only exists so a genuinely hung wrapper cannot
    /// wedge the suite; it is generous because process spawn under a fully parallel `cargo test`
    /// run has been observed to exceed a tight bound and fail the success-path assertions. ~keep
    const TEST_TIMEOUT_SECS: u64 = 120;

    fn session(working_directory: &Path, fingerprint: &str) -> ValidationSession {
        ValidationSession {
            language: Language::Kotlin,
            working_directory: working_directory.to_path_buf(),
            manifest: None,
            fingerprint: fingerprint.to_owned(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn only_gradle_manifest_file_names_are_recognized() {
        let cases = [
            ("build.gradle.kts", true),
            ("build.gradle", true),
            ("pubspec.yaml", false),
            ("Package.swift", false),
            ("local-fixture.jar", false),
        ];
        for (name, expected) in cases {
            assert_eq!(
                is_gradle_manifest(Path::new(name)),
                expected,
                "manifest file name {name}"
            );
        }
    }

    #[test]
    fn gradle_wrapper_is_found_only_when_present_at_the_project_root() {
        let root = tempfile::tempdir().expect("project root");

        assert!(gradle_wrapper(root.path()).is_none());

        std::fs::write(root.path().join(WRAPPER_NAME), "").expect("wrapper stub");
        assert_eq!(gradle_wrapper(root.path()), Some(root.path().join(WRAPPER_NAME)));
    }

    #[test]
    fn a_gradle_manifest_without_a_wrapper_falls_back_to_directory_probing() {
        let root = tempfile::tempdir().expect("project root");
        let classes = root.path().join("build/classes/kotlin/main");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "no-wrapper-fixture");

        let class_path = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("classpath resolves");

        assert_eq!(std::env::split_paths(&class_path).collect::<Vec<_>>(), vec![classes]);
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, contents).expect("script contents");
        let mut permissions = std::fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("script permissions");
    }

    #[cfg(unix)]
    #[test]
    fn a_gradle_wrapper_invocation_is_parsed_and_cached_across_repeated_resolutions() {
        let root = tempfile::tempdir().expect("project root");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        let invocation_log = root.path().join("invocations");
        write_executable(
            &root.path().join(WRAPPER_NAME),
            &format!(
                "#!/bin/sh\necho invoked >> {log}\necho \"noise: unrelated gradle output\"\necho \"ALEF_CLASSPATH_ENTRY:/fake/one.jar\"\necho \"ALEF_CLASSPATH_ENTRY:/fake/two.jar\"\n",
                log = invocation_log.display()
            ),
        );
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "wrapper-fixture");

        let first = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("first resolution");
        let second = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("second resolution");

        let expected = std::env::join_paths([PathBuf::from("/fake/one.jar"), PathBuf::from("/fake/two.jar")])
            .expect("expected classpath");
        assert_eq!(first, expected);
        assert_eq!(second, expected);
        let invocations = std::fs::read_to_string(&invocation_log).unwrap_or_default();
        assert_eq!(
            invocations.lines().count(),
            1,
            "the wrapper must run once and be served from cache thereafter: {invocations:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_gradle_wrapper_falls_back_to_directory_probing_instead_of_failing_the_session() {
        let root = tempfile::tempdir().expect("project root");
        let classes = root.path().join("build/classes/kotlin/main");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = root.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");
        write_executable(
            &root.path().join(WRAPPER_NAME),
            "#!/bin/sh\necho 'error: could not resolve dependencies' >&2\nexit 1\n",
        );
        let working_directory = tempfile::tempdir().expect("working directory");
        let session = session(working_directory.path(), "failing-wrapper-fixture");

        let class_path = resolve_class_path(&manifest, &session, TEST_TIMEOUT_SECS).expect("falls back to probing");

        assert_eq!(std::env::split_paths(&class_path).collect::<Vec<_>>(), vec![classes]);
    }
}
