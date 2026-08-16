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

    // `ruby -c` (the only check `validate` ever runs below `Run`, see above) is a syntax check: it
    // never resolves a constant, method, or class, and `run_script` sends it the identical `-c`
    // invocation for both `Syntax` and `Compile` — there is no separate compile step at all, so a
    // `Compile` request silently got the same result as `Syntax` while being reported as if it had
    // validated further. No real Ruby type-checker is wired up here either — Sorbet and RBS both
    // require project-wide `# typed:` sigils/signatures the generated snippets don't carry, so
    // pointing either at a lone temp file would either no-op (untyped files get minimal checking)
    // or hard-error on the missing project setup, neither of which is an improvement over an
    // honest downgrade. Until that's built, neither `compile` nor `typecheck` may be claimed. ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck) {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    // The compile/typecheck gap above is a property of this validator's implementation (no
    // distinct compile step and no checker is wired up), not of the machine running it — no
    // environment will ever make `ruby -c` resolve a constant. Structural, so it's exempted from
    // `Downgraded` the same way `max_level` is. ~keep
    fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
        matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck)
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
    fn achievable_level_caps_compile_and_typecheck_to_syntax() {
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Syntax
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

    #[test]
    fn achievable_level_compile_and_typecheck_gap_is_structural() {
        assert!(RubyValidator.achievable_level_is_structural(ValidationLevel::TypeCheck));
        assert!(RubyValidator.achievable_level_is_structural(ValidationLevel::Compile));
        assert!(!RubyValidator.achievable_level_is_structural(ValidationLevel::Run));
    }

    /// Mirrors the php.rs regression: `ruby -c` accepts this file (it never resolves constants),
    /// so `achievable_level` caps it to `syntax`; because that gap is structural, it is exempted
    /// from `Downgraded` the same way a `max_level` ceiling is — a capability-capped `Pass`, not
    /// a claim of `typecheck`. ~keep
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
        assert_eq!(result.status, SnippetStatus::Pass);
        assert!(result.capability_capped);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
    }

    /// The regression this fix closes: before `achievable_level` also capped `Compile`, this
    /// undefined-symbol snippet passed a `Compile` request as an ordinary, unqualified `Pass` —
    /// `ruby -c` accepts it, and nothing distinguished `Compile` from `Syntax`, so the result
    /// carried no `capability_capped` flag and no `downgrade_reason` at all. That is precisely the
    /// silent downgrade this validator must never produce again. ~keep
    #[test]
    fn compile_request_for_an_undefined_symbol_does_not_pass_as_compile() {
        if !RubyValidator.is_available() {
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
