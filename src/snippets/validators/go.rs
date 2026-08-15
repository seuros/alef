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
            Some(session) => tempfile::Builder::new()
                .prefix(".alef-snippet-")
                .tempdir_in(Self::project_directory(session))?,
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
        Self::apply_build_cache(&mut command, dir.path());
        Self::apply_dependency_caches(&mut command);
        match session {
            Some(value) => {
                command.current_dir(Self::project_directory(value));
                value.apply_environment(&mut command);
            }
            None => {
                command.current_dir(dir.path());
            }
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    /// Give the go toolchain a build cache inside the snippet's own temp directory.
    ///
    /// ~keep `go build`/`vet`/`run` refuse to start without one: `run_command`'s
    /// `sanitize_environment` allowlist carries neither `HOME` nor `GOCACHE`, and go then exits
    /// with "build cache is required, but could not be located: GOCACHE is not defined and $HOME
    /// is not defined" before compiling anything. Applied before `apply_environment` so a session
    /// that configures its own `GOCACHE` still wins and keeps its cache warm across snippets.
    fn apply_build_cache(command: &mut std::process::Command, dir: &std::path::Path) {
        command.env("GOCACHE", dir.join("go-build-cache"));
    }

    fn apply_dependency_caches(command: &mut std::process::Command) {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let (go_path, module_cache) =
            Self::dependency_cache_paths(std::env::var_os("GOPATH"), std::env::var_os("GOMODCACHE"), home);
        if let Some(path) = go_path {
            command.env("GOPATH", path);
        }
        if let Some(path) = module_cache {
            command.env("GOMODCACHE", path);
        }
    }

    fn dependency_cache_paths(
        go_path: Option<std::ffi::OsString>,
        module_cache: Option<std::ffi::OsString>,
        home: Option<std::ffi::OsString>,
    ) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
        let go_path = go_path
            .map(std::path::PathBuf::from)
            .or_else(|| home.as_deref().map(std::path::Path::new).map(|path| path.join("go")));
        let module_cache = module_cache
            .map(std::path::PathBuf::from)
            .or_else(|| go_path.as_ref().map(|path| path.join("pkg/mod")));
        (go_path, module_cache)
    }

    fn project_directory(session: &ValidationSession) -> &std::path::Path {
        session
            .manifest
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or(&session.working_directory)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn derives_dependency_caches_when_go_variables_are_not_exported() {
        let (go_path, module_cache) = GoValidator::dependency_cache_paths(None, None, Some("/home/sample".into()));

        assert_eq!(go_path, Some(PathBuf::from("/home/sample/go")));
        assert_eq!(module_cache, Some(PathBuf::from("/home/sample/go/pkg/mod")));
    }

    #[test]
    fn explicit_dependency_caches_override_home_defaults() {
        let (go_path, module_cache) = GoValidator::dependency_cache_paths(
            Some("/cache/go-path".into()),
            Some("/cache/modules".into()),
            Some("/home/sample".into()),
        );

        assert_eq!(go_path, Some(PathBuf::from("/cache/go-path")));
        assert_eq!(module_cache, Some(PathBuf::from("/cache/modules")));
    }

    #[test]
    fn compiles_a_snippet_under_the_sanitized_environment() {
        if which::which("go").is_err() {
            return;
        }
        let snippet = snippet("package main\n\nfunc main() {}\n");

        let (status, output) =
            GoValidator::validate_with_context(&snippet, ValidationLevel::Compile, TOOLCHAIN_TEST_TIMEOUT_SECS, None)
                .expect("validation runs");

        assert_eq!(
            status,
            SnippetStatus::Pass,
            "go must compile under the sanitized environment; with neither HOME nor GOCACHE it refuses to \
             start with \"build cache is required\" before compiling anything: {output:?}"
        );
    }

    #[test]
    fn session_manifest_resolves_a_local_module_outside_the_working_directory() {
        if which::which("go").is_err() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let working = root.path().join("working");
        let project = root.path().join("project");
        std::fs::create_dir_all(&working).expect("working directory");
        std::fs::create_dir_all(project.join("localpkg")).expect("local package directory");
        std::fs::write(project.join("go.mod"), "module example.test/local\n\ngo 1.24\n").expect("go manifest");
        std::fs::write(project.join("localpkg/value.go"), "package localpkg\nconst Value = 1\n").expect("go package");
        let snippet =
            snippet("package main\nimport \"example.test/local/localpkg\"\nfunc main() { _ = localpkg.Value }");
        let session = ValidationSession {
            working_directory: working,
            manifest: Some(project.join("go.mod")),
            fingerprint: "fixture".into(),
            env: BTreeMap::from([(
                "GOCACHE".into(),
                root.path().join("go-cache").to_string_lossy().into_owned(),
            )]),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = GoValidator::validate_with_context(
            &snippet,
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.go"),
            language: Language::Go,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.go"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
