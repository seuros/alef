use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};

pub struct PythonValidator;
const PYREFLY_UNAVAILABLE: &str = "pyrefly is not available for Python type-checking";

const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_CHECKER_NAME: &str = "alef_batch_check.py";
const BATCH_AST_MODE: &str = "ast";
const BATCH_COMPILE_MODE: &str = "compile";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the Python batch checker failed without a per-snippet diagnostic";
const BATCH_RUN_UNSUPPORTED: &str = "Python batch validation does not cover the run level";

/// One interpreter start for the whole batch instead of one per snippet: the checker walks every
/// file it is handed and emits one JSON line per file, so a syntax error in one snippet neither
/// aborts the run nor leaks into another snippet's result. It always exits 0, so the exit status
/// carries no per-snippet meaning and a missing line is what marks a snippet unjudged. ~keep
const BATCH_CHECKER_SOURCE: &str = r#"import ast
import json
import sys

mode = sys.argv[1]
for path in sys.argv[2:]:
    result = {"path": path, "ok": True, "error": ""}
    try:
        with open(path, encoding="utf-8") as handle:
            source = handle.read()
        if mode == "ast":
            ast.parse(source, filename=path)
        else:
            compile(source, path, "exec")
    except (SyntaxError, ValueError, UnicodeDecodeError, OSError) as error:
        result["ok"] = False
        result["error"] = "{}: {}".format(type(error).__name__, error)
    print(json.dumps(result))
"#;

impl PythonValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        if level == ValidationLevel::TypeCheck && which::which("pyrefly").is_err() {
            return Ok(vec![
                (SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string()));
                snippets.len()
            ]);
        }
        let dir = match session {
            Some(session) => session.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let mut paths = Vec::with_capacity(snippets.len());
        let mut file_names = Vec::with_capacity(snippets.len());
        for (index, snippet) in snippets.iter().enumerate() {
            let file_name = format!("{BATCH_FILE_PREFIX}{index}.py");
            let path = dir.path().join(&file_name);
            std::fs::write(&path, Self::patch_code(&snippet.code))?;
            paths.push(path);
            file_names.push(file_name);
        }
        let mut command = Self::batch_command(level, dir.path(), &paths)?;
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("PYTHONPATH", &session.working_directory);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(match level {
            ValidationLevel::TypeCheck => Self::typecheck_results(&file_names, success, &output),
            _ => Self::checker_results(&file_names, &output),
        })
    }

    fn batch_command(
        level: ValidationLevel,
        directory: &std::path::Path,
        paths: &[std::path::PathBuf],
    ) -> Result<std::process::Command> {
        if level == ValidationLevel::TypeCheck {
            let mut command = std::process::Command::new("pyrefly");
            command.arg("check").args(paths);
            return Ok(command);
        }
        let mode = match level {
            ValidationLevel::Syntax => BATCH_AST_MODE,
            ValidationLevel::Compile => BATCH_COMPILE_MODE,
            ValidationLevel::TypeCheck | ValidationLevel::Run => {
                return Err(crate::snippets::error::Error::Other(BATCH_RUN_UNSUPPORTED.to_string()));
            }
        };
        let checker_path = directory.join(BATCH_CHECKER_NAME);
        std::fs::write(&checker_path, BATCH_CHECKER_SOURCE)?;
        let mut command = std::process::Command::new(Self::interpreter());
        command.arg(&checker_path).arg(mode).args(paths);
        Ok(command)
    }

    /// Maps the batch checker's JSON lines back to the snippet that owns each file. A snippet the
    /// checker never reported on fails carrying the real output rather than passing by default —
    /// which is why the exit status is not consulted here: the checker reports every file it is
    /// given and exits 0 either way, so a missing line, not a non-zero exit, is what says a
    /// snippet went unjudged. ~keep
    fn checker_results(file_names: &[String], output: &str) -> BatchValidation {
        let mut reported = vec![None; file_names.len()];
        let mut unmatched = Vec::new();
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<serde_json::Value>(line).ok();
            let index = entry
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(serde_json::Value::as_str)
                .and_then(|path| Self::owner(file_names, path));
            match (index, entry) {
                (Some(index), Some(entry)) => {
                    let failed = entry.get("ok").and_then(serde_json::Value::as_bool) != Some(true);
                    let message = entry
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    reported[index] = Some(if failed {
                        (SnippetStatus::Fail, Some(message))
                    } else {
                        (SnippetStatus::Pass, None)
                    });
                }
                _ => unmatched.push(line.to_string()),
            }
        }
        let fallback = Self::fallback_message(reported.iter().all(Option::is_some), &unmatched);
        Self::finalize(reported, fallback)
    }

    /// Attributes `pyrefly check` diagnostics back to each file. Its full-text blocks start with an
    /// `ERROR` line and name the file on the following `-->` line; the one-line `min-text` form
    /// carries the path on the `ERROR` line itself, so both shapes are read. ~keep
    fn typecheck_results(file_names: &[String], success: bool, output: &str) -> BatchValidation {
        let mut blocks: Vec<Vec<String>> = Vec::new();
        let mut unmatched = Vec::new();
        let mut current: Option<Vec<String>> = None;
        for line in output.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("ERROR") {
                blocks.extend(current.take());
                current = Some(vec![line.to_string()]);
            } else if trimmed.starts_with("INFO") {
                blocks.extend(current.take());
            } else if let Some(block) = current.as_mut() {
                block.push(line.to_string());
            } else if !trimmed.is_empty() {
                unmatched.push(line.to_string());
            }
        }
        blocks.extend(current.take());

        let mut diagnostics = vec![Vec::new(); file_names.len()];
        for block in blocks {
            match Self::block_owner(file_names, &block) {
                Some(index) => diagnostics[index].push(block.join("\n")),
                None => unmatched.push(block.join("\n")),
            }
        }
        let attributed = diagnostics.iter().any(|messages| !messages.is_empty());
        let fallback = Self::fallback_message(success || attributed, &unmatched);
        let reported = diagnostics
            .into_iter()
            .map(|messages| (!messages.is_empty()).then(|| (SnippetStatus::Fail, Some(messages.join("\n")))))
            .collect();
        Self::finalize(reported, fallback)
    }

    fn fallback_message(resolved: bool, unmatched: &[String]) -> Option<String> {
        (!resolved).then(|| {
            if unmatched.is_empty() {
                BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
            } else {
                unmatched.join("\n")
            }
        })
    }

    fn finalize(reported: Vec<Option<(SnippetStatus, Option<String>)>>, fallback: Option<String>) -> BatchValidation {
        reported
            .into_iter()
            .map(|value| match (value, &fallback) {
                (Some(value), _) => value,
                (None, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (None, None) => (SnippetStatus::Pass, None),
            })
            .collect()
    }

    fn block_owner(file_names: &[String], block: &[String]) -> Option<usize> {
        block.iter().find_map(|line| {
            let candidate = match line.split_once("--> ") {
                Some((_, rest)) => rest,
                None => line.trim_start().strip_prefix("ERROR ")?,
            };
            let path = candidate.split(':').next()?;
            Self::owner(file_names, path.trim())
        })
    }

    fn owner(file_names: &[String], path: &str) -> Option<usize> {
        let name = std::path::Path::new(path).file_name()?;
        file_names
            .iter()
            .position(|file_name| std::ffi::OsStr::new(file_name.as_str()) == name)
    }

    fn interpreter() -> &'static str {
        if which::which("python3").is_ok() {
            "python3"
        } else {
            "python"
        }
    }

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
            Some(session) => session.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let code = Self::patch_code(&snippet.code);
        let snippet_path = dir.path().join("snippet.py");
        std::fs::write(&snippet_path, &code)?;
        let python = Self::interpreter();
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

    /// `Run` is declined: each snippet must execute in its own process so its output, exit status
    /// and side effects belong to it alone. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        (level != ValidationLevel::Run)
            .then(|| Self::validate_batch_with_context(snippets, level, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
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

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    fn python_snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("guide.md"),
            language: Language::Python,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("guide.md"),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn batch_declines_run_so_each_snippet_executes_on_its_own() {
        let only = python_snippet("value = 1\n");

        let declined = PythonValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 10, None);

        assert!(declined.is_none());
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !PythonValidator.is_available() {
            return;
        }
        let first = python_snippet("first = 1\n");
        let second = python_snippet("second = 2\n");
        let third = python_snippet("third = 3\n");

        let results = PythonValidator::validate_batch_with_context(
            &[&first, &second, &third],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None)
            ]
        );
    }

    #[test]
    fn batch_syntax_fails_only_the_broken_snippet_and_passes_its_neighbours() {
        if !PythonValidator.is_available() {
            return;
        }
        let first = python_snippet("value = 1\n");
        let broken = python_snippet("def broken(:\n    pass\n");
        let third = python_snippet("value = 3\n");

        let results = PythonValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[2], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("SyntaxError")),
            "the failing snippet must carry its own diagnostic: {:?}",
            results[1].1
        );
    }

    #[test]
    fn batch_compile_fails_only_the_broken_snippet() {
        if !PythonValidator.is_available() {
            return;
        }
        let first = python_snippet("value = 1\n");
        let broken = python_snippet("return 1\n");
        let third = python_snippet("value = 3\n");

        let results = PythonValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[2], (SnippetStatus::Pass, None));
    }

    #[test]
    fn batch_type_check_fails_only_the_snippet_pyrefly_names() {
        if which::which("pyrefly").is_err() {
            return;
        }
        let first = python_snippet("value: int = 1\nprint(value)\n");
        let broken = python_snippet("undefined_batch_name()\n");
        let third = python_snippet("other: int = 3\nprint(other)\n");

        let results = PythonValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("undefined_batch_name")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    #[test]
    fn batch_type_check_reports_every_snippet_unavailable_when_pyrefly_is_missing() {
        if which::which("pyrefly").is_ok() {
            return;
        }
        let first = python_snippet("value = 1\n");
        let second = python_snippet("value = 2\n");

        let results =
            PythonValidator::validate_batch_with_context(&[&first, &second], ValidationLevel::TypeCheck, 10, None)
                .expect("batch validation runs");

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string())),
                (SnippetStatus::Unavailable, Some(PYREFLY_UNAVAILABLE.to_string())),
            ]
        );
    }

    /// A checker that dies before reporting on a snippet must fail that snippet carrying the real
    /// output, never leave it passing by default. ~keep
    #[test]
    fn unreported_snippets_fail_with_the_real_output_when_the_checker_breaks() {
        let file_names = vec!["snippet_batch_0.py".to_string(), "snippet_batch_1.py".to_string()];
        let output = concat!(
            r#"{"path": "/tmp/x/snippet_batch_0.py", "ok": true, "error": ""}"#,
            "\nTraceback (most recent call last)\n"
        );

        let results = PythonValidator::checker_results(&file_names, output);

        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(
            results[1],
            (
                SnippetStatus::Fail,
                Some("Traceback (most recent call last)".to_string())
            )
        );
    }

    #[test]
    fn pyrefly_blocks_attach_to_the_file_named_on_their_location_line() {
        let file_names = vec!["snippet_batch_0.py".to_string(), "snippet_batch_1.py".to_string()];
        let output = concat!(
            "ERROR Could not find name `missing` [unknown-name]\n",
            " --> /tmp/x/snippet_batch_1.py:1:1\n",
            "  |\n",
            "1 | missing()\n",
            " INFO 1 error\n"
        );

        let results = PythonValidator::typecheck_results(&file_names, false, output);

        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("snippet_batch_1.py:1:1")),
            "{:?}",
            results[1].1
        );
    }

    #[test]
    fn a_type_checker_failure_naming_no_file_fails_every_snippet_with_the_real_output() {
        let file_names = vec!["snippet_batch_0.py".to_string(), "snippet_batch_1.py".to_string()];
        let output = "No `pyrefly.toml` found and the preset could not be resolved\n";

        let results = PythonValidator::typecheck_results(&file_names, false, output);

        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.0, SnippetStatus::Fail);
            assert_eq!(
                result.1.as_deref(),
                Some("No `pyrefly.toml` found and the preset could not be resolved")
            );
        }
    }

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
            language: Language::Python,
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

    /// Regression: `validate_with_context` used to create its session-scoped scratch directory
    /// directly inside `session.working_directory` via a bare `tempdir_in`, leaving a
    /// `.alef-snippet-*/` directory loose in a tracked package source directory after every run.
    /// It must nest under the session's own `.alef/snippets/tmp` cache root instead — and stay
    /// gone whether the snippet passes or fails. ~keep
    #[test]
    fn session_scratch_resolves_under_the_cache_root_and_is_removed_on_pass_and_fail() {
        if !PythonValidator.is_available() {
            return;
        }
        let directory = tempfile::tempdir().expect("temp directory");
        let session = ValidationSession {
            language: Language::Python,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "scratch-shape-fixture".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        };
        let passing = Snippet {
            id: None,
            path: "passing.py".into(),
            language: Language::Python,
            title: None,
            code: "value = 1\n".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "passing.py".into(),
                line: 1,
                block_index: 0,
            },
        };
        let mut failing = passing.clone();
        failing.code = "def broken(:\n".into();

        let (pass_status, pass_message) = PythonValidator
            .validate_in_session(&passing, ValidationLevel::Syntax, 10, Some(&session))
            .expect("passing snippet validates");
        assert_eq!(pass_status, SnippetStatus::Pass, "{pass_message:?}");
        let (fail_status, _) = PythonValidator
            .validate_in_session(&failing, ValidationLevel::Syntax, 10, Some(&session))
            .expect("failing snippet validates");
        assert_eq!(fail_status, SnippetStatus::Fail);

        let top_level_entries: Vec<_> = std::fs::read_dir(directory.path())
            .expect("read working directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect();
        assert!(
            top_level_entries.is_empty(),
            "no scratch entry may be left directly in working_directory: {top_level_entries:?}"
        );
        let scratch_root = directory.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a passing and a failing snippet validation"
        );
    }
}
