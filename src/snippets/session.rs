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

impl ValidationSession {
    pub fn workspace_directory(&self) -> Result<PathBuf> {
        let directory = self
            .working_directory
            .join(".alef/snippets/sessions")
            .join(&self.fingerprint);
        std::fs::create_dir_all(&directory)?;
        Ok(directory)
    }

    pub fn temp_dir(&self) -> Result<tempfile::TempDir> {
        tempfile::Builder::new()
            .prefix(".alef-snippet-")
            .tempdir_in(&self.working_directory)
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

pub fn prepare_sessions(
    specs: &HashMap<String, SessionSpec>,
    timeout_secs: u64,
) -> Result<HashMap<String, ValidationSession>> {
    specs
        .iter()
        .map(|(target, spec)| prepare_session(spec, timeout_secs).map(|session| (target.clone(), session)))
        .collect()
}

fn prepare_session(spec: &SessionSpec, timeout_secs: u64) -> Result<ValidationSession> {
    let language = spec.language;
    ensure_directory(&spec.working_directory, language)?;
    if let Some(manifest) = &spec.manifest
        && !manifest.is_file()
    {
        return Err(Error::Other(format!(
            "configured {language} snippet manifest does not exist: {}",
            manifest.display()
        )));
    }
    for command in &spec.before {
        run_before(command, &spec.working_directory, &spec.env, timeout_secs)
            .map_err(|error| Error::Other(format!("preparing {language} snippet validation session: {error}")))?;
    }
    let session = ValidationSession {
        working_directory: spec.working_directory.clone(),
        manifest: spec.manifest.clone(),
        fingerprint: session_fingerprint(spec)?,
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

        let sessions = prepare_sessions(&specs, 5).expect("session preparation succeeds");

        assert!(marker.exists());
        assert_eq!(sessions.len(), 1);
    }

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

        let error = prepare_sessions(&specs, 5).expect_err("missing manifest is rejected");

        assert!(error.to_string().contains("manifest does not exist"));
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

        let sessions = prepare_sessions(&specs, 5).expect("session preparation succeeds");
        let session = sessions.get("zig").expect("zig session");
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

#[cfg(windows)]
fn shell_command(source: &str) -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    command.args(["/C", source]);
    command
}
