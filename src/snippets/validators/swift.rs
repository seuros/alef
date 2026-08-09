use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct SwiftValidator;

impl SnippetValidator for SwiftValidator {
    fn language(&self) -> Language {
        Language::Swift
    }

    fn is_available(&self) -> bool {
        which::which("swiftc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let file = dir.path().join("snippet.swift");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("swiftc");
        match level {
            ValidationLevel::Syntax => {
                command.args(["-parse"]).arg(&file);
            }
            ValidationLevel::Compile => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
            }
            ValidationLevel::TypeCheck => {
                command.args(["-typecheck", "-warnings-as-errors"]).arg(&file);
            }
            ValidationLevel::Run => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
            }
        }

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::TypeCheck
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let Some(session) = session else {
            return self.validate(snippet, level, timeout_secs);
        };
        let dir = session.temp_dir()?;
        let file = dir.path().join("snippet.swift");
        std::fs::write(&file, snippet.code.trim())?;
        let module_directory = swift_module_directory(session)?;
        let mut command = std::process::Command::new("swiftc");
        match level {
            ValidationLevel::Syntax => {
                command.arg("-parse");
            }
            ValidationLevel::TypeCheck => {
                command.args(["-typecheck", "-warnings-as-errors"]);
            }
            ValidationLevel::Compile => {
                command.arg("-typecheck");
            }
            ValidationLevel::Run => {
                command.arg("-o").arg(dir.path().join("snippet"));
            }
        }
        command
            .args([
                "-I",
                module_directory.to_string_lossy().as_ref(),
                "-L",
                module_directory
                    .parent()
                    .unwrap_or(&module_directory)
                    .to_string_lossy()
                    .as_ref(),
            ])
            .arg(&file);
        session.apply(&mut command);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("no such module") || output.contains("cannot find") && output.contains("in scope")
    }
}

fn swift_module_directory(session: &ValidationSession) -> Result<std::path::PathBuf> {
    let mut command = std::process::Command::new("swift");
    command.args(["build", "--show-bin-path"]);
    session.apply(&mut command);
    let output = command.output()?;
    if !output.status.success() {
        return Err(crate::snippets::error::Error::Other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let binary_directory = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    Ok(binary_directory.join("Modules"))
}
