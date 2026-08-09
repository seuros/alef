use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct JavaValidator;

impl JavaValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = match session {
            Some(value) => value.temp_dir()?,
            None => TempDir::new()?,
        };
        let wrapped = Self::wrap_if_fragment(&snippet.code);
        let class_name = Self::extract_class_name(&wrapped);
        let file = dir.path().join(format!("{class_name}.java"));
        std::fs::write(&file, &wrapped)?;
        let mut command = if level == ValidationLevel::Run {
            let mut value = std::process::Command::new("java");
            value.arg(&file);
            value
        } else {
            let mut value = std::process::Command::new("javac");
            value.arg(if level == ValidationLevel::TypeCheck {
                "-Xlint:all"
            } else {
                "-Xlint:none"
            });
            if level == ValidationLevel::TypeCheck {
                value.arg("-Werror");
            }
            value.args(["-d"]).arg(dir.path()).arg(&file);
            value
        };
        if let Some(value) = session {
            value.apply(&mut command);
            if let Some(manifest) = &value.manifest {
                command.args(["--class-path", manifest.to_string_lossy().as_ref()]);
            }
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn extract_class_name(code: &str) -> String {
        for line in code.lines() {
            let trimmed = line.trim();
            for keyword in ["public class ", "class ", "public final class ", "final class "] {
                if let Some(rest) = trimmed.strip_prefix(keyword) {
                    let name = rest
                        .split(|c: char| c.is_whitespace() || c == '{' || c == '<')
                        .next()
                        .unwrap_or("Snippet");
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
        "Snippet".to_string()
    }

    fn has_class_or_interface(code: &str) -> bool {
        code.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("class ")
                || trimmed.starts_with("public class ")
                || trimmed.starts_with("final class ")
                || trimmed.starts_with("public final class ")
                || trimmed.starts_with("interface ")
                || trimmed.starts_with("public interface ")
                || trimmed.starts_with("public enum ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("public record ")
                || trimmed.starts_with("record ")
        })
    }

    fn split_imports(code: &str) -> (String, String) {
        let mut imports = Vec::new();
        let mut body = Vec::new();
        let mut past_imports = false;
        for line in code.lines() {
            let trimmed = line.trim();
            if !past_imports
                && (trimmed.is_empty() || trimmed.starts_with("import ") || trimmed.starts_with("package "))
            {
                imports.push(line);
            } else {
                past_imports = true;
                body.push(line);
            }
        }
        (imports.join("\n"), body.join("\n"))
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if Self::has_class_or_interface(trimmed) {
            return code.to_string();
        }

        let (imports, body) = Self::split_imports(trimmed);
        let body_trimmed = body.trim();
        let only_comments = !body_trimmed.is_empty()
            && body_trimmed
                .lines()
                .all(|line| line.trim().is_empty() || line.trim().starts_with("//"));

        let body_inner = if body_trimmed.is_empty() || only_comments {
            format!("{body_trimmed}\nint _placeholder = 0;")
        } else {
            body.to_string()
        };
        let imports_block = if imports.trim().is_empty() {
            String::new()
        } else {
            format!("{imports}\n\n")
        };
        format!(
            "{imports_block}public class Snippet {{\n    public static void main(String[] args) throws Exception {{\n{body_inner}\n    }}\n}}\n"
        )
    }
}

impl SnippetValidator for JavaValidator {
    fn language(&self) -> Language {
        Language::Java
    }

    fn is_available(&self) -> bool {
        which::which("javac").is_ok()
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
        ValidationLevel::TypeCheck
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("cannot find symbol") || output.contains("package") && output.contains("does not exist")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn session_manifest_is_used_as_a_real_classpath() {
        if which::which("javac").is_err() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let classes = root.path().join("classes");
        let sources = root.path().join("sources/localfixture");
        std::fs::create_dir_all(&classes).expect("classes directory");
        std::fs::create_dir_all(&sources).expect("sources directory");
        let source = sources.join("Values.java");
        std::fs::write(
            &source,
            "package localfixture; public final class Values { public static final int VALUE = 1; }",
        )
        .expect("Java fixture source");
        let compiled = std::process::Command::new("javac")
            .args(["-d"])
            .arg(&classes)
            .arg(&source)
            .status()
            .expect("javac runs");
        assert!(compiled.success());
        let session = ValidationSession {
            working_directory: root.path().to_path_buf(),
            manifest: Some(classes),
            fingerprint: "fixture".into(),
            env: BTreeMap::new(),
        };

        let (status, output) = JavaValidator::validate_with_context(
            &snippet("import localfixture.Values;\npublic final class Example { public static void main(String[] args) { System.out.println(Values.VALUE); } }"),
            ValidationLevel::TypeCheck,
            30,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("Example.java"),
            language: Language::Java,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("Example.java"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
