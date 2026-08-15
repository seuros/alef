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

    // `parse(file = ...)` (the only check this validator ever runs below `Run`, see
    // `validate_with_context` above) is R's pure syntax parser: it never resolves a function or
    // package. Real static checkers (`codetools::checkUsage`, lintr) do exist, but aren't wired
    // up here, so `typecheck` must not be claimed until they are. ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if requested == ValidationLevel::TypeCheck {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::runner::{RunnerConfig, run_validation};
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use crate::snippets::validators::ValidatorRegistry;

    fn undefined_function_snippet() -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language: Language::R,
            title: None,
            code: "thisFunctionDoesNotExist12345()\n".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "example.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn achievable_level_caps_typecheck_to_syntax() {
        assert_eq!(RValidator.achievable_level(ValidationLevel::TypeCheck), ValidationLevel::Syntax);
        assert_eq!(RValidator.achievable_level(ValidationLevel::Compile), ValidationLevel::Run);
        assert_eq!(RValidator.achievable_level(ValidationLevel::Syntax), ValidationLevel::Run);
        assert_eq!(RValidator.achievable_level(ValidationLevel::Run), ValidationLevel::Run);
    }

    /// A snippet that is syntactically valid but references a function that cannot resolve must
    /// not come back as a `typecheck` pass. Before `achievable_level`, `parse(file = ...)`
    /// accepted this file (it never resolves functions) and the runner reported it Pass at
    /// `effective_level: typecheck` because `max_level` was `Run` and nothing downgraded it — the
    /// exact false green this test pins shut. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_function_does_not_pass_as_typecheck() {
        if !RValidator.is_available() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary =
            run_validation(&[undefined_function_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::TypeCheck),
            "undefined-function snippet must not pass claiming typecheck: {result:?}"
        );
        assert_eq!(result.status, SnippetStatus::Downgraded);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 1);
    }
}
