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
