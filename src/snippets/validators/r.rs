use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::NamedTempFile;

pub struct RValidator;

impl RValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let mut source = match session {
            Some(value) => tempfile::Builder::new()
                .suffix(".R")
                .tempfile_in(&value.working_directory)?,
            None => NamedTempFile::with_suffix(".R")?,
        };
        source.write_all(snippet.code.as_bytes())?;
        source.flush()?;
        let mut command = std::process::Command::new("Rscript");
        if level == ValidationLevel::Run {
            command.arg(source.path());
        } else {
            command.args(["-e", &format!("parse(file = {:?})", source.path().to_string_lossy())]);
        }
        if let Some(value) = session {
            value.apply(&mut command);
            command.env("R_LIBS_USER", &value.working_directory);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }
}

impl SnippetValidator for RValidator {
    fn language(&self) -> Language {
        Language::R
    }

    fn is_available(&self) -> bool {
        which::which("Rscript").is_ok() || which::which("R").is_ok()
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
        output.contains("could not find function")
            || output.contains("there is no package called")
            || output.contains("cannot open file")
    }
}
