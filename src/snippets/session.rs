use crate::snippets::error::{Error, Result};
use crate::snippets::types::Language;
use crate::snippets::validators::run_command;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct SessionSpec {
    pub working_directory: PathBuf,
    pub manifest: Option<PathBuf>,
    pub before: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ValidationSession {
    pub working_directory: PathBuf,
    pub manifest: Option<PathBuf>,
    pub fingerprint: String,
    pub env: BTreeMap<String, String>,
}

impl ValidationSession {
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
        command.envs(&self.env);
    }
}

pub fn prepare_sessions(
    specs: &HashMap<Language, SessionSpec>,
    timeout_secs: u64,
) -> Result<HashMap<Language, ValidationSession>> {
    specs
        .iter()
        .map(|(language, spec)| prepare_session(*language, spec, timeout_secs).map(|session| (*language, session)))
        .collect()
}

fn prepare_session(language: Language, spec: &SessionSpec, timeout_secs: u64) -> Result<ValidationSession> {
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
    Ok(ValidationSession {
        working_directory: spec.working_directory.clone(),
        manifest: spec.manifest.clone(),
        fingerprint: session_fingerprint(spec)?,
        env: spec.env.clone(),
    })
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
            Language::Python,
            SessionSpec {
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec![format!("test ! -e prepared && touch {}", marker.display())],
                env: BTreeMap::new(),
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
            Language::TypeScript,
            SessionSpec {
                working_directory: directory.path().to_path_buf(),
                manifest: Some(directory.path().join("missing.json")),
                before: Vec::new(),
                env: BTreeMap::new(),
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
            Language::Zig,
            SessionSpec {
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: vec!["test \"$ALEF_SESSION_CACHE\" = configured".into()],
                env: BTreeMap::from([("ALEF_SESSION_CACHE".into(), "configured".into())]),
            },
        );

        let sessions = prepare_sessions(&specs, 5).expect("session preparation succeeds");
        let session = sessions.get(&Language::Zig).expect("zig session");
        let mut command = std::process::Command::new("true");
        session.apply(&mut command);

        assert_eq!(
            command.get_envs().next(),
            Some(("ALEF_SESSION_CACHE".as_ref(), Some("configured".as_ref())))
        );
    }
}

#[cfg(windows)]
fn shell_command(source: &str) -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    command.args(["/C", source]);
    command
}
