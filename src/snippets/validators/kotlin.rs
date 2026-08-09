use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct KotlinValidator;

impl KotlinValidator {
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
        let file = dir.path().join("snippet.kt");
        std::fs::write(&file, snippet.code.trim())?;
        let mut command = std::process::Command::new("kotlinc");
        if level == ValidationLevel::TypeCheck {
            command.arg("-Werror");
        }
        if level == ValidationLevel::Run {
            command.arg("-include-runtime");
        } else {
            command.arg("-nowarn");
        }
        if let Some(manifest) = session.and_then(|value| value.manifest.as_ref()) {
            command.args(["-classpath", manifest.to_string_lossy().as_ref()]);
        }
        command
            .arg("-d")
            .arg(if level == ValidationLevel::Run {
                dir.path().join("out.jar")
            } else {
                dir.path().join("out")
            })
            .arg(&file);
        if let Some(value) = session {
            value.apply(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }
}

impl SnippetValidator for KotlinValidator {
    fn language(&self) -> Language {
        Language::Kotlin
    }

    fn is_available(&self) -> bool {
        which::which("kotlinc").is_ok()
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
        output.contains("unresolved reference") || output.contains("expecting an element")
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
        if which::which("kotlinc").is_err() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let source = root.path().join("LocalFixture.kt");
        let library = root.path().join("local-fixture.jar");
        std::fs::write(
            &source,
            "package localfixture\nobject Values { const val value: Int = 1 }\n",
        )
        .expect("Kotlin fixture source");
        let compiled = std::process::Command::new("kotlinc")
            .arg(&source)
            .args(["-d"])
            .arg(&library)
            .status()
            .expect("kotlinc runs");
        assert!(compiled.success());
        let session = ValidationSession {
            working_directory: root.path().to_path_buf(),
            manifest: Some(library),
            fingerprint: "fixture".into(),
            env: BTreeMap::new(),
        };

        let (status, output) = KotlinValidator::validate_with_context(
            &snippet("import localfixture.Values\nfun main() { println(Values.value) }"),
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
            path: PathBuf::from("snippet.kt"),
            language: Language::Kotlin,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.kt"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
