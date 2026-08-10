use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::TempDir;

pub struct RustValidator;

impl RustValidator {
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
        let source_dir = dir.path().join("src");
        std::fs::create_dir_all(&source_dir)?;
        std::fs::write(dir.path().join("Cargo.toml"), Self::cargo_manifest(session)?)?;
        let code = Self::wrap_if_fragment(&snippet.code);
        let mut source_file = std::fs::File::create(source_dir.join("main.rs"))?;
        source_file.write_all(code.as_bytes())?;
        let args: &[&str] = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile | ValidationLevel::TypeCheck => &["check", "--quiet"],
            ValidationLevel::Run => &["run", "--quiet"],
        };
        let mut command = std::process::Command::new("cargo");
        command.args(args).current_dir(dir.path());
        if let Some(session) = session {
            session.apply_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn cargo_manifest(session: Option<&ValidationSession>) -> Result<String> {
        let dependency = session.map(Self::path_dependency).transpose()?.unwrap_or_default();
        let dependencies = session
            .map(Self::additional_dependencies)
            .transpose()?
            .unwrap_or_default();
        Ok(format!(
            "[workspace]\n\n[package]\nname = \"snippet-check\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependency}{dependencies}"
        ))
    }

    fn path_dependency(session: &ValidationSession) -> Result<String> {
        let manifest = session
            .manifest
            .clone()
            .unwrap_or_else(|| session.working_directory.join("Cargo.toml"));
        let content = std::fs::read_to_string(&manifest)?;
        let value: toml::Value = toml::from_str(&content).map_err(|error| {
            crate::snippets::error::Error::Other(format!("parsing {}: {error}", manifest.display()))
        })?;
        let package = value
            .get("package")
            .and_then(|value| value.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                crate::snippets::error::Error::Other(format!("no package.name in {}", manifest.display()))
            })?;
        let crate_name = package.replace('-', "_");
        Ok(format!(
            "{crate_name} = {{ package = {package:?}, path = {:?}, features = {:?} }}\n",
            manifest
                .parent()
                .unwrap_or(&session.working_directory)
                .to_string_lossy(),
            session.rust_features
        ))
    }

    fn additional_dependencies(session: &ValidationSession) -> Result<String> {
        let mut dependencies = String::new();
        for (name, dependency) in &session.rust_dependencies {
            if !Self::valid_dependency_name(name) {
                return Err(crate::snippets::error::Error::Other(format!(
                    "invalid Rust snippet dependency name `{name}`"
                )));
            }
            let mut specification = toml::map::Map::new();
            specification.insert("version".into(), toml::Value::String(dependency.version.clone()));
            specification.insert(
                "features".into(),
                toml::Value::Array(dependency.features.iter().cloned().map(toml::Value::String).collect()),
            );
            specification.insert(
                "default-features".into(),
                toml::Value::Boolean(dependency.default_features),
            );
            dependencies.push_str(&format!("{name} = {}\n", toml::Value::Table(specification)));
        }
        Ok(dependencies)
    }

    fn valid_dependency_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }

    fn is_bare_signature(code: &str) -> bool {
        let trimmed = code.trim();
        trimmed.contains("fn ") && !trimmed.contains('{')
    }

    fn has_use_then_statements(code: &str) -> bool {
        let trimmed = code.trim();
        if !trimmed.starts_with("use ") {
            return false;
        }

        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with("use ") {
                continue;
            }

            if line.starts_with("let ")
                || line.starts_with("println!")
                || line.starts_with("eprintln!")
                || line.starts_with("assert")
                || line.starts_with("if ")
                || line.starts_with("for ")
                || line.starts_with("while ")
                || line.starts_with("match ")
                || line.starts_with("loop ")
                || line.starts_with("tokio::")
                || line.starts_with("std::")
                || line.starts_with("//")
            {
                return true;
            }

            return false;
        }

        false
    }

    fn split_uses(code: &str) -> (String, String) {
        let mut uses = Vec::new();
        let mut body = Vec::new();
        let mut past_uses = false;

        for line in code.lines() {
            let trimmed = line.trim();
            if !past_uses && (trimmed.starts_with("use ") || trimmed.is_empty()) {
                uses.push(line);
            } else {
                past_uses = true;
                body.push(line);
            }
        }

        (uses.join("\n"), body.join("\n"))
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if trimmed.contains("fn main()") {
            return code.to_string();
        }

        if Self::is_bare_signature(trimmed) {
            return format!("{code}\n\nfn main() {{}}");
        }

        if Self::has_use_then_statements(code) {
            let (uses, body) = Self::split_uses(code);
            return format!("{uses}\n\nfn main() {{\n{body}\n}}");
        }

        let has_top_level_items = trimmed.starts_with("use ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("enum ")
            || trimmed.starts_with("impl ")
            || trimmed.starts_with("mod ")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("const ")
            || trimmed.starts_with("static ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("extern ")
            || trimmed.starts_with("unsafe ");

        if has_top_level_items {
            format!("{code}\n\nfn main() {{}}")
        } else {
            format!("fn main() {{\n{code}\n}}")
        }
    }
}

impl SnippetValidator for RustValidator {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn is_available(&self) -> bool {
        which::which("cargo").is_ok()
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
            "E0432", "E0433", "E0412", "E0405", "E0425", "E0463", "E0277", "E0599", "E0752", "E0308", "E0107", "E0609",
            "E0061", "E0574", "E0583", "E0282", "E0728", "E0423",
        ];

        let error_lines: Vec<&str> = output
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("error")
                    || trimmed.contains("aborting due to")
                    || trimmed.starts_with("Some errors have")
                    || trimmed.starts_with("For more information")
            })
            .collect();

        if error_lines.is_empty() {
            return false;
        }

        error_lines.iter().any(|line| {
            patterns.iter().any(|pattern| line.contains(pattern))
                || line.contains("unresolved import")
                || line.contains("cannot find")
                || line.contains("not found in")
                || line.contains("could not compile")
                || line.contains("derive macro")
                || line.contains("proc-macro")
                || line.contains("main function not found")
                || line.contains("functions are not allowed in")
                || line.contains("expected one of")
                || line.contains("expected parameter name")
                || line.contains("not allowed to be `async`")
                || line.contains("expected item, found")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;

    #[test]
    fn session_manifest_links_the_configured_local_crate() {
        if which::which("cargo").is_err() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        std::fs::create_dir_all(project.path().join("src")).expect("source directory");
        let manifest = project.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"sample-binding\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("package manifest");
        std::fs::write(project.path().join("src/lib.rs"), "pub const VALUE: usize = 1;\n").expect("package source");
        let session = ValidationSession {
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let snippet = Snippet {
            id: None,
            path: "snippet.rs".into(),
            language: Language::Rust,
            title: None,
            code: "fn main() { assert_eq!(sample_binding::VALUE, 1); }".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "snippet.rs".into(),
                line: 1,
                block_index: 0,
            },
        };

        let (status, output) =
            RustValidator::validate_with_context(&snippet, ValidationLevel::TypeCheck, 30, Some(&session))
                .expect("validation runs");

        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    #[test]
    fn session_dependency_enables_declared_features() {
        let project = tempfile::tempdir().expect("project directory");
        let manifest = project.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"sample-binding\"\nversion = \"0.1.0\"\n\n[features]\ndefault = []\nnetwork = []\n",
        )
        .expect("package manifest");
        let session = ValidationSession {
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            rust_features: vec!["network".into()],
            rust_dependencies: BTreeMap::new(),
        };

        let dependency = RustValidator::path_dependency(&session).expect("dependency");
        assert!(dependency.contains("features = [\"network\"]"));
    }

    #[test]
    fn session_manifest_adds_explicit_dependencies() {
        let mut session = ValidationSession {
            working_directory: std::path::PathBuf::from("fixture"),
            manifest: None,
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        session.rust_dependencies.insert(
            "async-runtime".into(),
            crate::core::config::output::DocsSnippetRustDependencyConfig {
                version: "1".into(),
                features: vec!["macros".into()],
                default_features: false,
            },
        );

        let dependencies = RustValidator::additional_dependencies(&session).expect("dependencies");
        assert!(dependencies.starts_with("async-runtime = {"));
        assert!(dependencies.contains("version = \"1\""));
        assert!(dependencies.contains("features = [\"macros\"]"));
        assert!(dependencies.contains("default-features = false"));
    }

    #[test]
    fn rejects_invalid_dependency_names() {
        let mut session = ValidationSession {
            working_directory: std::path::PathBuf::from("fixture"),
            manifest: None,
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        session.rust_dependencies.insert(
            "invalid\n[package]".into(),
            crate::core::config::output::DocsSnippetRustDependencyConfig {
                version: "1".into(),
                features: Vec::new(),
                default_features: true,
            },
        );

        assert!(RustValidator::additional_dependencies(&session).is_err());
    }
}
