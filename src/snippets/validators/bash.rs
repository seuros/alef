use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_script};

pub struct BashValidator;

impl SnippetValidator for BashValidator {
    fn language(&self) -> Language {
        Language::Bash
    }

    fn is_available(&self) -> bool {
        which::which("bash").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        run_script(snippet, level, timeout_secs, None, ".sh", "bash", &["-n"])
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        run_script(snippet, level, timeout_secs, session, ".sh", "bash", &["-n"])
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    // `bash -n` (the only check this validator ever runs below `Run`, see `validate` above) is a
    // parse-only check: it never resolves a command, builtin, or function. A real checker
    // (ShellCheck) does exist, but isn't wired up here, so `typecheck` must not be claimed until
    // it is. ~keep
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

    fn undefined_command_snippet() -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language: Language::Bash,
            title: None,
            code: "this_command_does_not_exist_12345\n".into(),
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
        assert_eq!(
            BashValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            BashValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Run
        );
        assert_eq!(
            BashValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(
            BashValidator.achievable_level(ValidationLevel::Run),
            ValidationLevel::Run
        );
    }

    /// A snippet that is syntactically valid but references a command that cannot exist must not
    /// come back as a `typecheck` pass. Before `achievable_level`, `bash -n` accepted this file
    /// (it never resolves commands) and the runner reported it Pass at `effective_level: typecheck`
    /// because `max_level` was `Run` and nothing downgraded it — the exact false green this test
    /// pins shut. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_command_does_not_pass_as_typecheck() {
        if !BashValidator.is_available() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[undefined_command_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::TypeCheck),
            "undefined-command snippet must not pass claiming typecheck: {result:?}"
        );
        assert_eq!(result.status, SnippetStatus::Downgraded);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 1);
    }
}
