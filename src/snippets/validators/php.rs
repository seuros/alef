use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_script};

pub struct PhpValidator;

impl SnippetValidator for PhpValidator {
    fn language(&self) -> Language {
        Language::Php
    }

    fn is_available(&self) -> bool {
        which::which("php").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        run_script(snippet, level, timeout_secs, None, ".php", "php", &["-l"])
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        run_script(snippet, level, timeout_secs, session, ".php", "php", &["-l"])
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    // `php -l` (the only check this validator ever runs below `Run`, see `validate` above) is a
    // syntax check: it never resolves a class, function, or constant. No real PHP type-checker
    // (PHPStan, Psalm) is wired up here, because a correct call needs the project's composer
    // autoload path to avoid flagging every legitimately external symbol as unresolvable — a
    // false-fail regression, not a fix. Until that's built, `typecheck` must not be claimed. ~keep
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

    fn undefined_symbol_snippet() -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language: Language::Php,
            title: None,
            code: "<?php\n$bogus = new ThisClassDoesNotExistAnywhere12345();\n".into(),
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
            PhpValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            PhpValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Run
        );
        assert_eq!(
            PhpValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(
            PhpValidator.achievable_level(ValidationLevel::Run),
            ValidationLevel::Run
        );
    }

    /// A snippet that is syntactically valid but references a symbol that cannot exist must not
    /// come back as a `typecheck` pass. Before `achievable_level`, `php -l` accepted this file
    /// (it never resolves classes) and the runner reported it Pass at `effective_level: typecheck`
    /// because `max_level` was `Run` and nothing downgraded it — the exact false green this test
    /// pins shut. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_symbol_does_not_pass_as_typecheck() {
        if !PhpValidator.is_available() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[undefined_symbol_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::TypeCheck),
            "undefined-symbol snippet must not pass claiming typecheck: {result:?}"
        );
        assert_eq!(result.status, SnippetStatus::Downgraded);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 1);
    }
}
