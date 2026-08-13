use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct PythonValidator;
const PYREFLY_UNAVAILABLE: &str = "pyrefly is not available for Python type-checking";

impl PythonValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if level == ValidationLevel::TypeCheck && which::which("pyrefly").is_err() {
            return Ok((SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string())));
        }
        let dir = match session {
            Some(session) => tempfile::Builder::new()
                .prefix(".alef-snippet-")
                .tempdir_in(&session.working_directory)?,
            None => TempDir::new()?,
        };
        let code = Self::patch_code(&snippet.code);
        let snippet_path = dir.path().join("snippet.py");
        std::fs::write(&snippet_path, &code)?;
        let python = if which::which("python3").is_ok() {
            "python3"
        } else {
            "python"
        };
        let path = snippet_path.to_string_lossy().to_string();
        let mut command = Self::command(level, dir.path(), python, &path)?;
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("PYTHONPATH", &session.working_directory);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn command(
        level: ValidationLevel,
        directory: &std::path::Path,
        python: &str,
        path: &str,
    ) -> Result<std::process::Command> {
        let command = match level {
            ValidationLevel::Syntax => {
                let checker_path = directory.join("check.py");
                std::fs::write(&checker_path, "import ast, sys\nast.parse(open(sys.argv[1]).read())\n")?;
                let mut command = std::process::Command::new(python);
                command.args([checker_path.to_string_lossy().as_ref(), path]);
                command
            }
            ValidationLevel::Compile => {
                let mut command = std::process::Command::new(python);
                command.args(["-m", "py_compile", path]);
                command
            }
            ValidationLevel::TypeCheck => {
                let mut command = std::process::Command::new("pyrefly");
                command.args(["check", path]);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new(python);
                command.arg(path);
                command
            }
        };
        Ok(command)
    }

    fn patch_code(code: &str) -> String {
        let trimmed = code.trim();

        if trimmed.starts_with(' ') || trimmed.starts_with('\t') {
            let min_indent = trimmed
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.len() - line.trim_start().len())
                .min()
                .unwrap_or(0);

            if min_indent > 0 {
                let dedented = trimmed
                    .lines()
                    .map(|line| {
                        if line.trim().is_empty() {
                            String::new()
                        } else if line.len() > min_indent {
                            line[min_indent..].to_string()
                        } else {
                            line.trim().to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                return Self::patch_signatures(&dedented);
            }
        }

        Self::patch_signatures(code)
    }

    fn patch_signatures(code: &str) -> String {
        let lines: Vec<&str> = code.lines().collect();
        let mut output = Vec::new();
        let mut index = 0;

        while index < lines.len() {
            output.push(lines[index].to_string());
            let trimmed = lines[index].trim();
            let is_def_start =
                trimmed.starts_with("def ") || trimmed.starts_with("async def ") || trimmed.starts_with("class ");

            if is_def_start {
                let mut signature_end = index;
                let mut has_inline_body = false;

                while signature_end < lines.len() {
                    let trimmed_signature = lines[signature_end].trim();
                    if signature_end > index {
                        output.push(lines[signature_end].to_string());
                    }

                    if trimmed_signature.ends_with(':') {
                        break;
                    }

                    if let Some(arrow_pos) = trimmed_signature.find("->") {
                        let after_arrow = &trimmed_signature[arrow_pos + 2..];
                        if let Some(colon_pos) = after_arrow.find(':') {
                            let after_colon = after_arrow[colon_pos + 1..].trim();
                            if !after_colon.is_empty() {
                                has_inline_body = true;
                            }
                            break;
                        }

                        if signature_end == index {
                            let last = output.len() - 1;
                            output[last] = format!("{}:", lines[signature_end]);
                        }
                        break;
                    }

                    if trimmed_signature.contains("): ") || trimmed_signature.contains("):\t") {
                        has_inline_body = true;
                        break;
                    }

                    if trimmed_signature.ends_with(')') && signature_end > index {
                        let last = output.len() - 1;
                        output[last] = format!("{}:", output[last]);
                        break;
                    }

                    signature_end += 1;
                }

                if signature_end >= lines.len() {
                    let last = output.len() - 1;
                    if !output[last].trim().ends_with(':') {
                        output[last] = format!("{}:", output[last]);
                    }
                    let indent = lines[index].chars().take_while(|c| c.is_whitespace()).count();
                    output.push(format!("{}...", " ".repeat(indent + 4)));
                    index = signature_end;
                    continue;
                }

                if has_inline_body {
                    index = signature_end + 1;
                    continue;
                }

                let next_content = (signature_end + 1..lines.len())
                    .find(|candidate| !lines[*candidate].trim().is_empty())
                    .map(|candidate| lines[candidate]);

                let has_body = next_content.is_some_and(|line| line.starts_with(' ') || line.starts_with('\t'));
                if !has_body {
                    let last = output.len() - 1;
                    if !output[last].trim().ends_with(':') {
                        output[last] = format!("{}:", output[last]);
                    }

                    let indent = lines[index].chars().take_while(|c| c.is_whitespace()).count();
                    output.push(format!("{}...", " ".repeat(indent + 4)));
                }

                index = signature_end + 1;
                continue;
            }

            index += 1;
        }

        output.join("\n")
    }
}

impl SnippetValidator for PythonValidator {
    fn language(&self) -> Language {
        Language::Python
    }

    fn is_available(&self) -> bool {
        which::which("python3").is_ok() || which::which("python").is_ok()
    }

    fn is_available_at(&self, level: ValidationLevel) -> bool {
        if level != ValidationLevel::TypeCheck {
            return self.is_available();
        }
        which::which("pyrefly").is_ok()
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
        let _ = output;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{PYREFLY_UNAVAILABLE, PythonValidator};
    use crate::snippets::session::ValidationSession;
    use crate::snippets::types::{Language, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel};
    use crate::snippets::validators::SnippetValidator;
    use std::path::PathBuf;

    #[test]
    fn pyrefly_command_matches_scaffolded_python_tooling() {
        let command = PythonValidator::command(
            ValidationLevel::TypeCheck,
            std::path::Path::new("."),
            "python3",
            "snippet.py",
        )
        .expect("type-check command");
        assert_eq!(command.get_program(), "pyrefly");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["check", "snippet.py"]);
    }

    #[test]
    fn unavailable_diagnostic_names_only_the_supported_checker() {
        assert_eq!(PYREFLY_UNAVAILABLE, "pyrefly is not available for Python type-checking");
        assert!(!PYREFLY_UNAVAILABLE.contains("mypy"));
    }

    #[test]
    fn preserves_multiline_async_signature_lines() {
        let code = r"class UserServiceHandler:
    async def CreateUsers(
        self, request_iterator
    ) -> CreateUsersResponse:
        created_users = []
        return created_users
";

        let patched = PythonValidator::patch_code(code);
        assert!(patched.contains(") -> CreateUsersResponse:"));
        assert!(patched.contains("created_users = []"));
    }

    #[test]
    fn syntax_validation_rejects_malformed_imports_and_indentation() {
        let path = PathBuf::from("broken.py");
        let snippet = Snippet {
            id: None,
            path: path.clone(),
            language: Language::Python,
            title: None,
            code: "from sample import call    from sample.types import Request\n  result = call()".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path,
                line: 1,
                block_index: 0,
            },
        };

        let (status, _) = PythonValidator
            .validate(&snippet, ValidationLevel::Syntax, 10)
            .expect("syntax validator runs");
        assert_eq!(status, SnippetStatus::Fail);
    }

    #[test]
    fn run_session_resolves_local_binding_from_working_directory() {
        if !PythonValidator.is_available() {
            return;
        }
        let directory = tempfile::tempdir().expect("temp directory");
        std::fs::write(directory.path().join("local_binding.py"), "VALUE = 42\n").expect("local binding");
        let path = PathBuf::from("local.py");
        let snippet = Snippet {
            id: None,
            path: path.clone(),
            language: Language::Python,
            title: None,
            code: "import local_binding\nassert local_binding.VALUE == 42\n".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path,
                line: 1,
                block_index: 0,
            },
        };
        let session = ValidationSession {
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "test-binding".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        };

        let (status, message) = PythonValidator
            .validate_in_session(&snippet, ValidationLevel::Run, 10, Some(&session))
            .expect("session validation runs");

        assert_eq!(status, SnippetStatus::Pass, "{message:?}");
    }
}
