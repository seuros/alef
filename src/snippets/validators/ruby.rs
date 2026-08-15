use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_script};

pub struct RubyValidator;

impl SnippetValidator for RubyValidator {
    fn language(&self) -> Language {
        Language::Ruby
    }

    fn is_available(&self) -> bool {
        which::which("ruby").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if is_api_signature(snippet.code.trim()) {
            return Ok((SnippetStatus::Pass, None));
        }

        run_script(snippet, level, timeout_secs, None, ".rb", "ruby", &["-c"])
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if is_api_signature(snippet.code.trim()) {
            return Ok((SnippetStatus::Pass, None));
        }
        run_script(snippet, level, timeout_secs, session, ".rb", "ruby", &["-c"])
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    // `ruby -c` (the only check this validator ever runs below `Run`, see `validate` above) is a
    // syntax check: it never resolves a constant, method, or class. No real Ruby type-checker is
    // wired up here — Sorbet and RBS both require project-wide `# typed:` sigils/signatures the
    // generated snippets don't carry, so pointing either at a lone temp file would either no-op
    // (untyped files get minimal checking) or hard-error on the missing project setup, neither of
    // which is an improvement over an honest downgrade. Until that's built, `typecheck` must not
    // be claimed. ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if requested == ValidationLevel::TypeCheck {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }
}

fn is_api_signature(code: &str) -> bool {
    code.lines().count() <= 3 && code.contains(" -> ")
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
            language: Language::Ruby,
            title: None,
            code: "ThisConstantDoesNotExistAnywhere12345.call_me\n".into(),
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
            RubyValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Run
        );
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::Run),
            ValidationLevel::Run
        );
    }

    /// Mirrors the php.rs regression: `ruby -c` accepts this file (it never resolves constants),
    /// so before `achievable_level` capped the level, the runner reported it Pass at
    /// `effective_level: typecheck` — the false green this test pins shut. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_symbol_does_not_pass_as_typecheck() {
        if !RubyValidator.is_available() {
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
