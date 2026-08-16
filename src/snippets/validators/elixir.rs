use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct ElixirValidator;

impl SnippetValidator for ElixirValidator {
    fn language(&self) -> Language {
        Language::Elixir
    }

    fn is_available(&self) -> bool {
        which::which("elixir").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let snippet_path = dir.path().join("snippet.exs");
        std::fs::write(&snippet_path, &snippet.code)?;

        let mut command = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile | ValidationLevel::TypeCheck => {
                let checker_path = dir.path().join("check.exs");
                let checker = format!(
                    r#"path = "{}"
case File.read(path) do
  {{:ok, content}} ->
    case Code.string_to_quoted(content) do
      {{:ok, _}} -> System.halt(0)
      {{:error, reason}} ->
        IO.puts("parse error: #{{inspect(reason)}}")
        System.halt(1)
    end
  {{:error, reason}} ->
    IO.puts("file read error: #{{inspect(reason)}}")
    System.halt(1)
end"#,
                    snippet_path.to_string_lossy().replace('\\', "\\\\")
                );
                std::fs::write(&checker_path, checker)?;

                let mut command = std::process::Command::new("elixir");
                command.arg(checker_path);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new("elixir");
                command.arg(&snippet_path);
                command
            }
        };

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    // `Code.string_to_quoted` (the only check run at `Syntax`/`Compile`/`TypeCheck`, see the
    // match above and in `validate_in_session` below) parses the AST and nothing more — Elixir
    // resolves remote calls like `Mod.fun()` at runtime, not parse time, so a made-up module
    // parses cleanly. No real check is wired up here: `mix dialyzer` needs an out-of-band PLT
    // this harness doesn't build (first run can take minutes, easily blowing the per-snippet
    // timeout), and compiling with `elixirc` can't tell a genuinely undefined module from one
    // that's legitimately defined elsewhere on a code path this isolated snippet doesn't have —
    // that would trade the false-pass this fixes for false-fails on correct code. Until a real
    // checker is wired up with proper project/PLT context, `typecheck` must not be claimed. ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if requested == ValidationLevel::TypeCheck {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    // The typecheck gap above is a property of this validator's implementation (no checker is
    // wired up), not of the machine running it — no environment will ever make
    // `Code.string_to_quoted` resolve a remote call. Structural, so it's exempted from
    // `Downgraded` the same way `max_level` is. ~keep
    fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
        requested == ValidationLevel::TypeCheck
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
        let file = dir.path().join("snippet.exs");
        std::fs::write(&file, &snippet.code)?;
        let mut command = if level == ValidationLevel::Run {
            let mut value = std::process::Command::new("elixir");
            value.arg(&file);
            value
        } else {
            let mut value = std::process::Command::new("elixir");
            value.args([
                "-e",
                &format!("Code.string_to_quoted!(File.read!({:?}))", file.to_string_lossy()),
            ]);
            value
        };
        session.apply(&mut command);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
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
            language: Language::Elixir,
            title: None,
            code: "ThisModuleDoesNotExistAnywhere12345.call_me()\n".into(),
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
            ElixirValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            ElixirValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Run
        );
        assert_eq!(
            ElixirValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(
            ElixirValidator.achievable_level(ValidationLevel::Run),
            ValidationLevel::Run
        );
    }

    #[test]
    fn achievable_level_typecheck_gap_is_structural() {
        assert!(ElixirValidator.achievable_level_is_structural(ValidationLevel::TypeCheck));
        assert!(!ElixirValidator.achievable_level_is_structural(ValidationLevel::Compile));
        assert!(!ElixirValidator.achievable_level_is_structural(ValidationLevel::Run));
    }

    /// Mirrors the php.rs/ruby.rs regressions: `Code.string_to_quoted` accepts this file (Elixir
    /// resolves remote calls at runtime, not parse time), so `achievable_level` caps it to
    /// `syntax`; because that gap is structural, it is exempted from `Downgraded` the same way a
    /// `max_level` ceiling is — a capability-capped `Pass`, not a claim of `typecheck`. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_symbol_does_not_pass_as_typecheck() {
        if !ElixirValidator.is_available() {
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
}
