use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};

pub struct DartValidator;

impl DartValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = match session {
            Some(value) => ScratchDir::for_session(value)?,
            None => ScratchDir::isolated()?,
        };
        let file = dir.path().join("snippet.dart");
        std::fs::write(&file, snippet.code.trim())?;
        let mut command = std::process::Command::new("dart");
        match level {
            ValidationLevel::Syntax => {
                command.args(["analyze", "--no-fatal-warnings"]).arg(&file);
            }
            ValidationLevel::Compile => {
                command
                    .args(["compile", "exe", "-o"])
                    .arg(dir.path().join("snippet.aot"))
                    .arg(&file);
            }
            ValidationLevel::TypeCheck => {
                command.args(["analyze", "--fatal-infos"]).arg(&file);
            }
            ValidationLevel::Run => {
                command.arg("run").arg(&file);
            }
        }
        if let Some(value) = session {
            command.current_dir(Self::project_directory(value));
            value.apply_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn project_directory(session: &ValidationSession) -> &std::path::Path {
        session
            .manifest
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or(&session.working_directory)
    }
}

impl SnippetValidator for DartValidator {
    fn language(&self) -> Language {
        Language::Dart
    }

    fn is_available(&self) -> bool {
        which::which("dart").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, None)
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, session)
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("uri_does_not_exist") || output.contains("undefined_identifier")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn session_manifest_resolves_a_local_package_outside_the_working_directory() {
        if which::which("dart").is_err() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let working = root.path().join("working");
        let project = root.path().join("project");
        std::fs::create_dir_all(&working).expect("working directory");
        std::fs::create_dir_all(project.join("lib")).expect("library directory");
        std::fs::write(
            project.join("pubspec.yaml"),
            "name: local_fixture\nenvironment:\n  sdk: '>=3.0.0 <4.0.0'\n",
        )
        .expect("Dart manifest");
        std::fs::write(project.join("lib/local_fixture.dart"), "const value = 1;\n").expect("Dart library");
        let prepared = std::process::Command::new("dart")
            .args(["pub", "get"])
            .current_dir(&project)
            .status()
            .expect("dart pub get runs");
        assert!(prepared.success());
        let snippet = snippet("import 'package:local_fixture/local_fixture.dart';\nvoid main() { print(value); }");
        let session = ValidationSession {
            language: Language::Dart,
            working_directory: working,
            manifest: Some(project.join("pubspec.yaml")),
            fingerprint: "fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) =
            DartValidator::validate_with_context(&snippet, ValidationLevel::TypeCheck, 30, Some(&session))
                .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    fn scratch_shape_session(project: &std::path::Path, fingerprint: &str) -> ValidationSession {
        ValidationSession {
            language: Language::Dart,
            working_directory: project.to_path_buf(),
            manifest: None,
            fingerprint: fingerprint.into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    fn scratch_top_level_entries(project: &std::path::Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(project)
            .expect("read project directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect()
    }

    /// Regression: `validate_with_context` used to create its session-scoped scratch directory
    /// directly inside `project_directory(session)` (a tracked package source directory) via a
    /// bare `tempdir_in`, leaving a `.alef-snippet-*/` directory loose in `packages/dart/` after
    /// every run. It must nest under that project's own `.alef/snippets/tmp` cache root instead. ~keep
    #[test]
    fn session_scratch_resolves_under_the_cache_root_not_the_project_directory() {
        if which::which("dart").is_err() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-shape-fixture");
        let snippet = snippet("void main() { print('ok'); }\n");

        let (status, output) =
            DartValidator::validate_with_context(&snippet, ValidationLevel::Syntax, 30, Some(&session))
                .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory: {leftovers:?}"
        );
    }

    /// Pins cleanup on the failure path specifically: a snippet that fails `dart analyze` must
    /// not leave its scratch directory behind under the project directory any more than a
    /// passing one does.
    #[test]
    fn session_scratch_is_removed_after_a_run_that_fails() {
        if which::which("dart").is_err() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-cleanup-fixture");
        let snippet = snippet("this does not parse as dart {{{\n");

        let (status, _) = DartValidator::validate_with_context(&snippet, ValidationLevel::Syntax, 30, Some(&session))
            .expect("validation runs");
        assert_eq!(status, SnippetStatus::Fail);

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory after a failing run: {leftovers:?}"
        );
        let scratch_root = project.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a failing snippet validation"
        );
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.dart"),
            language: Language::Dart,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.dart"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
