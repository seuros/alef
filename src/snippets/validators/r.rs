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

    // `parse(file = ...)` (the only check `validate_with_context` ever runs below `Run`, see
    // above) is R's pure syntax parser: it never resolves a function or package, and it is sent
    // identically for both `Syntax` and `Compile` — there is no separate compile step at all, so
    // a `Compile` request silently got the same result as `Syntax` while being reported as if it
    // had validated further. Real static checkers (`codetools::checkUsage`, lintr) do exist, but
    // aren't wired up here, so neither `compile` nor `typecheck` may be claimed until they are.
    // ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck) {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    // The compile/typecheck gap above is a property of this validator's implementation (no
    // distinct compile step and no checker is wired up), not of the machine running it — no
    // environment will ever make `parse(file = ...)` resolve a function. Structural, so it's
    // exempted from `Downgraded` the same way `max_level` is. ~keep
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
    fn achievable_level_caps_compile_and_typecheck_to_syntax() {
        assert_eq!(
            RValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(RValidator.achievable_level(ValidationLevel::Run), ValidationLevel::Run);
    }

    #[test]
    fn achievable_level_compile_and_typecheck_gap_is_structural() {
        assert!(RValidator.achievable_level_is_structural(ValidationLevel::TypeCheck));
        assert!(RValidator.achievable_level_is_structural(ValidationLevel::Compile));
        assert!(!RValidator.achievable_level_is_structural(ValidationLevel::Run));
    }

    /// A snippet that is syntactically valid but references a function that cannot resolve must
    /// not come back as a `typecheck` pass. `parse(file = ...)` accepts this file (it never
    /// resolves functions), so `achievable_level` caps it to `syntax`; because that gap is
    /// structural, it is exempted from `Downgraded` the same way a `max_level` ceiling is — a
    /// capability-capped `Pass`, not a claim of `typecheck`. ~keep
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
        assert_eq!(result.status, SnippetStatus::Pass);
        assert!(result.capability_capped);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
    }

    /// The regression this fix closes: before `achievable_level` also capped `Compile`, this
    /// undefined-function snippet passed a `Compile` request as an ordinary, unqualified `Pass` —
    /// `parse(file = ...)` accepts it, and nothing distinguished `Compile` from `Syntax`, so the
    /// result carried no `capability_capped` flag and no `downgrade_reason` at all. That is
    /// precisely the silent downgrade this validator must never produce again. ~keep
    #[test]
    fn compile_request_for_an_undefined_function_does_not_pass_as_compile() {
        if !RValidator.is_available() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary =
            run_validation(&[undefined_function_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::Compile),
            "undefined-function snippet must not pass claiming compile: {result:?}"
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
