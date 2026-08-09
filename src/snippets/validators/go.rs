use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct GoValidator;

impl GoValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = match session {
            Some(session) => session.temp_dir()?,
            None => TempDir::new()?,
        };
        let file = dir.path().join("snippet.go");
        std::fs::write(&file, Self::wrap_if_fragment(&snippet.code))?;
        if session.is_none() && level != ValidationLevel::Syntax {
            std::fs::write(dir.path().join("go.mod"), "module snippet\n\ngo 1.21\n")?;
        }
        let mut command = match level {
            ValidationLevel::Syntax => {
                let mut command = std::process::Command::new("gofmt");
                command.args(["-e", "-l"]).arg(&file);
                command
            }
            ValidationLevel::Compile => {
                let mut command = std::process::Command::new("go");
                command.args(["build", "-o", "/dev/null"]).arg(&file);
                command
            }
            ValidationLevel::TypeCheck => {
                let mut command = std::process::Command::new("go");
                command.arg("vet").arg(&file);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new("go");
                command.arg("run").arg(&file);
                command
            }
        };
        command.current_dir(session.map_or(dir.path(), |value| &value.working_directory));
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if trimmed.starts_with("package ") {
            return code.to_string();
        }

        let (imports, body) = Self::split_imports(trimmed);
        let body_trimmed = body.trim();
        let only_comments = !body_trimmed.is_empty()
            && body_trimmed
                .lines()
                .all(|line| line.trim().is_empty() || line.trim().starts_with("//"));

        if body_trimmed.is_empty() || only_comments {
            let imports_block = if imports.trim().is_empty() {
                String::new()
            } else {
                format!("{imports}\n\n")
            };
            return format!("package main\n\n{imports_block}func main() {{\n{body_trimmed}\n_ = 0\n}}\n");
        }

        let imports_block = if imports.trim().is_empty() {
            String::new()
        } else {
            format!("{imports}\n\n")
        };
        format!("package main\n\n{imports_block}func main() {{\n{body}\n}}\n")
    }

    fn split_imports(code: &str) -> (String, String) {
        let mut imports = Vec::new();
        let mut body = Vec::new();
        let mut lines = code.lines().peekable();

        while let Some(line) = lines.peek() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                imports.push(*line);
                lines.next();
                continue;
            }
            if trimmed.starts_with("import (") {
                for import_line in lines.by_ref() {
                    imports.push(import_line);
                    if import_line.trim() == ")" {
                        break;
                    }
                }
                continue;
            }
            if let Some(stripped) = trimmed.strip_prefix("import ") {
                let stripped = stripped.trim();
                if stripped.starts_with('"') || stripped.starts_with('`') {
                    imports.push(*line);
                    lines.next();
                    continue;
                }
            }
            break;
        }
        for line in lines {
            body.push(line);
        }
        (imports.join("\n"), body.join("\n"))
    }
}

impl SnippetValidator for GoValidator {
    fn language(&self) -> Language {
        Language::Go
    }

    fn is_available(&self) -> bool {
        which::which("go").is_ok() || which::which("gofmt").is_ok()
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
        output.contains("undefined:") || output.contains("cannot find package") || output.contains("no required module")
    }
}
