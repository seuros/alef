use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct DartValidator;

impl DartValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = match session {
            Some(value) => tempfile::Builder::new()
                .prefix(".alef-snippet-")
                .tempdir_in(Self::project_directory(value))?,
            None => TempDir::new()?,
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
            working_directory: working,
            manifest: Some(project.join("pubspec.yaml")),
            fingerprint: "fixture".into(),
            env: BTreeMap::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) =
            DartValidator::validate_with_context(&snippet, ValidationLevel::TypeCheck, 30, Some(&session))
                .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
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
