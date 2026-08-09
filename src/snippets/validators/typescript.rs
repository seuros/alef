use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::TempDir;

pub struct TypeScriptValidator;

impl TypeScriptValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if Self::is_api_signature(&snippet.code)
            || snippet.code.trim().starts_with("!!!")
            || snippet.code.trim().starts_with("???")
        {
            return Ok((SnippetStatus::Pass, None));
        }
        let dir = match session {
            Some(session) => tempfile::Builder::new()
                .prefix(".alef-snippet-")
                .tempdir_in(&session.working_directory)?,
            None => TempDir::new()?,
        };
        if session.is_none() {
            std::fs::write(dir.path().join("tsconfig.json"), Self::isolated_tsconfig())?;
        }
        let file_path = dir.path().join("snippet.ts");
        let mut file = std::fs::File::create(&file_path)?;
        file.write_all(Self::dedent(&snippet.code).as_bytes())?;
        let project = session
            .and_then(|value| value.manifest.as_ref())
            .map(|manifest| Self::write_overlay_config(dir.path(), manifest))
            .transpose()?;
        let mut command = Self::command(level, &file_path, dir.path(), session, project.as_deref());
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn isolated_tsconfig() -> &'static str {
        r#"{"compilerOptions":{"strict":true,"noEmit":true,"target":"ES2022","module":"ES2022","moduleResolution":"bundler","skipLibCheck":true},"include":["*.ts"]}"#
    }

    fn write_overlay_config(directory: &std::path::Path, manifest: &std::path::Path) -> Result<std::path::PathBuf> {
        let path = directory.join("tsconfig.json");
        let content = serde_json::json!({ "extends": manifest, "files": ["snippet.ts"] });
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&content).map_err(|error| {
                crate::snippets::error::Error::Other(format!("serializing TypeScript snippet config: {error}"))
            })?,
        )?;
        Ok(path)
    }

    fn command(
        level: ValidationLevel,
        file_path: &std::path::Path,
        isolated_directory: &std::path::Path,
        session: Option<&ValidationSession>,
        project: Option<&std::path::Path>,
    ) -> std::process::Command {
        let mut command = if level == ValidationLevel::Run {
            let mut command = std::process::Command::new("tsx");
            if let Some(project) = project {
                command.args(["--tsconfig", project.to_string_lossy().as_ref()]);
            }
            command.arg(file_path);
            command
        } else {
            let mut command = std::process::Command::new("tsc");
            command.args(["--noEmit", "--pretty", "false"]);
            if level == ValidationLevel::Syntax {
                command.arg("--noCheck");
            }
            if let Some(project) = project {
                command.args(["--project", project.to_string_lossy().as_ref()]);
            } else {
                command.current_dir(isolated_directory);
            }
            command
        };
        if let Some(session) = session {
            command.current_dir(&session.working_directory);
        }
        command
    }

    fn dedent(code: &str) -> String {
        let min_indent = code
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.len() - line.trim_start().len())
            .min()
            .unwrap_or(0);

        if min_indent == 0 {
            return code.to_string();
        }

        code.lines()
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
            .join("\n")
    }

    fn is_api_signature(code: &str) -> bool {
        let trimmed = code.trim();

        if trimmed.lines().count() <= 6 {
            let has_fn_decl = trimmed.starts_with("function ")
                || trimmed.starts_with("async function ")
                || trimmed.starts_with("export function ")
                || trimmed.starts_with("export async function ");
            return has_fn_decl && !trimmed.contains('{');
        }

        false
    }
}

impl SnippetValidator for TypeScriptValidator {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn is_available(&self) -> bool {
        which::which("tsc").is_ok()
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
        let patterns = [
            "TS2307", "TS2304", "TS2305", "TS2306", "TS2322", "TS2345", "TS2339", "TS2351", "TS2552", "TS2314",
            "TS2391", "TS2693", "TS7016", "TS2371", "TS2580", "TS1375", "TS2792", "TS2503", "TS7006", "TS2769",
            "TS1128", "TS1005", "TS18046", "TS18047", "TS2531", "TS2532", "TS2451",
        ];

        let error_lines: Vec<&str> = output.lines().filter(|line| line.contains("error TS")).collect();
        if error_lines.is_empty() {
            return false;
        }

        error_lines
            .iter()
            .all(|line| patterns.iter().any(|pattern| line.contains(pattern)))
    }
}
