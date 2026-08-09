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
            Some(value) => value.temp_dir()?,
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
            value.apply(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
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
