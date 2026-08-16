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

    // `php -l` (the only check `validate` ever runs below `Run`, see above) is a syntax check: it
    // never resolves a class, function, or constant, and `run_script` sends it the identical `-l`
    // invocation for both `Syntax` and `Compile` — there is no separate compile step at all, so a
    // `Compile` request silently got the same result as `Syntax` while being reported as if it had
    // validated further. No real PHP type-checker (PHPStan, Psalm) is wired up here either, because
    // a correct call needs the project's composer autoload path to avoid flagging every legitimately
    // external symbol as unresolvable — a false-fail regression, not a fix. Until that's built,
    // neither `compile` nor `typecheck` may be claimed. ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck) {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    // The compile/typecheck gap above is a property of this validator's implementation (no
    // distinct compile step and no checker is wired up), not of the machine running it — no
    // environment will ever make `php -l` resolve a class. Structural, so it's exempted from
    // `Downgraded` the same way `max_level` is. ~keep
    fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
        matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck)
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
    fn achievable_level_caps_compile_and_typecheck_to_syntax() {
        assert_eq!(
            PhpValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            PhpValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Syntax
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

    #[test]
    fn achievable_level_compile_and_typecheck_gap_is_structural() {
        assert!(PhpValidator.achievable_level_is_structural(ValidationLevel::TypeCheck));
        assert!(PhpValidator.achievable_level_is_structural(ValidationLevel::Compile));
        assert!(!PhpValidator.achievable_level_is_structural(ValidationLevel::Run));
    }

    /// A snippet that is syntactically valid but references a symbol that cannot exist must not
    /// come back as a `typecheck` pass. `php -l` accepts this file (it never resolves classes),
    /// so `achievable_level` caps it to `syntax`; because that gap is structural (see
    /// `achievable_level_is_structural`), it is exempted from `Downgraded` the same way a
    /// `max_level` ceiling is — a capability-capped `Pass`, not a claim of `typecheck`. ~keep
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
        assert_eq!(result.status, SnippetStatus::Pass);
        assert!(result.capability_capped);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
    }

    /// The regression this fix closes: before `achievable_level` also capped `Compile`, this
    /// undefined-symbol snippet passed a `Compile` request as an ordinary, unqualified `Pass` —
    /// `php -l` accepts it, and nothing distinguished `Compile` from `Syntax`, so the result
    /// carried no `capability_capped` flag and no `downgrade_reason` at all. That is precisely the
    /// silent downgrade this validator must never produce again: `php -l` never resolves a class
    /// regardless of the level requested, so a `Compile` request must land here too. ~keep
    #[test]
    fn compile_request_for_an_undefined_symbol_does_not_pass_as_compile() {
        if !PhpValidator.is_available() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[undefined_symbol_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::Compile),
            "undefined-symbol snippet must not pass claiming compile: {result:?}"
        );
        assert_eq!(result.status, SnippetStatus::Pass);
        assert!(
            result.capability_capped,
            "a Compile request that only ran a syntax check must be flagged, not folded into an \
             ordinary Pass: {result:?}"
        );
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
    }
}
