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
            let class_path = Self::class_path(manifest)?;
            command.args(["-classpath", class_path.to_string_lossy().as_ref()]);
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

    fn class_path(manifest: &std::path::Path) -> Result<std::ffi::OsString> {
        if manifest.is_dir() || manifest.extension().is_some_and(|extension| extension == "jar") {
            return Ok(manifest.as_os_str().to_owned());
        }
        let root = manifest.parent().unwrap_or_else(|| std::path::Path::new("."));
        let build = root.join("build");
        let mut entries = [
            build.join("classes/kotlin/main"),
            build.join("classes/java/main"),
            build.join("intermediates/javac/debug/classes"),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
        let libraries = build.join("libs");
        if libraries.is_dir() {
            entries.extend(
                std::fs::read_dir(libraries)?
                    .filter_map(std::result::Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|extension| extension == "jar")),
            );
        }
        if entries.is_empty() {
            entries.push(root.to_path_buf());
        }
        std::env::join_paths(entries).map_err(|error| {
            crate::snippets::error::Error::Other(format!(
                "building Kotlin classpath for {}: {error}",
                manifest.display()
            ))
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

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

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
            .status();
        let compiled = match compiled {
            Ok(status) => status,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("kotlinc runs: {error}"),
        };
        assert!(compiled.success());
        let session = ValidationSession {
            working_directory: root.path().to_path_buf(),
            manifest: Some(library),
            fingerprint: "fixture".into(),
            env: BTreeMap::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = KotlinValidator::validate_with_context(
            &snippet("import localfixture.Values\nfun main() { println(Values.value) }"),
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    #[test]
    fn build_manifest_resolves_compiled_class_directory() {
        let project = tempfile::tempdir().expect("project directory");
        let classes = project.path().join("build/classes/kotlin/main");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = project.path().join("build.gradle.kts");
        std::fs::write(&manifest, "plugins {}").expect("manifest");

        let class_path = KotlinValidator::class_path(&manifest).expect("classpath");
        assert_eq!(std::env::split_paths(&class_path).collect::<Vec<_>>(), vec![classes]);
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
