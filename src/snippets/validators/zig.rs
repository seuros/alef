use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct ZigValidator;

impl SnippetValidator for ZigValidator {
    fn language(&self) -> Language {
        Language::Zig
    }

    fn is_available(&self) -> bool {
        which::which("zig").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let file = dir.path().join("snippet.zig");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("zig");
        match level {
            ValidationLevel::Syntax => {
                command.arg("ast-check").arg(&file);
            }
            ValidationLevel::Compile | ValidationLevel::TypeCheck | ValidationLevel::Run => {
                command.args(["build-exe", "-fno-emit-bin"]).arg(&file);
            }
        }
        apply_cache_dirs(&mut command, dir.path());

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Compile
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
        let file = dir.path().join("snippet.zig");
        std::fs::write(&file, snippet.code.trim())?;
        let mut command = std::process::Command::new("zig");
        if level == ValidationLevel::Syntax {
            command.arg("ast-check");
        } else {
            command.args(["build-exe", "-fno-emit-bin"]);
        }
        if level == ValidationLevel::Syntax {
            command.arg(&file);
        } else if let Some(manifest) = session.manifest.as_deref() {
            let (module_name, module_source) = zig_package_module(manifest)?;
            command
                .args(["--dep", &module_name])
                .arg(format!("-Mroot={}", file.display()))
                .arg(format!("-M{module_name}={}", module_source.display()));
        } else {
            command.arg(&file);
        }
        apply_include_paths(&mut command, &session.include_paths);
        apply_cache_dirs(&mut command, dir.path());
        session.apply(&mut command);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("unable to find") || output.contains("@import")
    }
}

/// Point zig's caches inside the snippet's own temp directory.
///
/// ~keep zig resolves its cache directory from `HOME`/`XDG_CACHE_HOME`, and `run_command`'s
/// `sanitize_environment` allowlist carries neither. Without these variables zig aborts with
/// `error: unable to resolve zig cache directory: AppDataDirUnavailable` before it reads a single
/// line of the snippet, so every zig snippet fails identically at compile level and the failure
/// looks like a defect in the snippet. Setting them explicitly keeps the run hermetic instead of
/// widening the allowlist, which would leak the developer's real zig cache into validation.
fn apply_cache_dirs(command: &mut std::process::Command, dir: &std::path::Path) {
    command.env("ZIG_GLOBAL_CACHE_DIR", dir.join("zig-global-cache"));
    command.env("ZIG_LOCAL_CACHE_DIR", dir.join("zig-local-cache"));
}

fn apply_include_paths(command: &mut std::process::Command, include_paths: &[std::path::PathBuf]) {
    for include_path in include_paths {
        command.arg("-I").arg(include_path);
    }
}

fn zig_package_module(manifest: &std::path::Path) -> Result<(String, std::path::PathBuf)> {
    let source = std::fs::read_to_string(manifest)?;
    let module_marker = "addModule(\"";
    let module_start = source.find(module_marker).ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("no addModule declaration in {}", manifest.display()))
    })? + module_marker.len();
    let module_end = source[module_start..].find('"').ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("invalid addModule declaration in {}", manifest.display()))
    })? + module_start;
    let root_marker = "root_source_file = b.path(\"";
    let root_start = source[module_end..].find(root_marker).ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("no module root source in {}", manifest.display()))
    })? + module_end
        + root_marker.len();
    let root_end = source[root_start..].find('"').ok_or_else(|| {
        crate::snippets::error::Error::Other(format!("invalid module root source in {}", manifest.display()))
    })? + root_start;
    let root = manifest
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(&source[root_start..root_end]);
    Ok((source[module_start..module_end].to_owned(), root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SnippetStatus, SourceOrigin};
    use std::path::PathBuf;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn compiles_a_snippet_under_the_sanitized_environment() {
        if which::which("zig").is_err() {
            return;
        }
        let snippet =
            zig_snippet("const std = @import(\"std\");\n\npub fn main() void {\n    _ = std.mem.zeroes(u8);\n}\n");

        let (status, output) = ZigValidator
            .validate(&snippet, ValidationLevel::Compile, TOOLCHAIN_TEST_TIMEOUT_SECS)
            .expect("validation runs");

        assert_eq!(
            status,
            SnippetStatus::Pass,
            "zig must compile under the sanitized environment; without an explicit cache directory it \
             fails with AppDataDirUnavailable before reading the snippet: {output:?}"
        );
    }

    #[test]
    fn cache_directories_are_scoped_to_the_snippet_directory() {
        let root = tempfile::tempdir().expect("temporary root");
        let mut command = std::process::Command::new("zig");
        apply_cache_dirs(&mut command, root.path());

        let configured: Vec<_> = command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key.to_string_lossy().into_owned(), PathBuf::from(value))))
            .collect();

        assert_eq!(
            configured,
            vec![
                ("ZIG_GLOBAL_CACHE_DIR".to_string(), root.path().join("zig-global-cache")),
                ("ZIG_LOCAL_CACHE_DIR".to_string(), root.path().join("zig-local-cache")),
            ]
        );
    }

    fn zig_snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.zig"),
            language: Language::Zig,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.zig"),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn resolves_declared_package_module() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(
            &manifest,
            "const module = b.addModule(\"sample_binding\", .{\n    .root_source_file = b.path(\"src/root.zig\"),\n});\n",
        )
        .unwrap();
        let (name, source) = zig_package_module(&manifest).unwrap();
        assert_eq!(name, "sample_binding");
        assert_eq!(source, directory.path().join("src/root.zig"));
    }

    #[test]
    fn session_include_paths_are_passed_to_zig() {
        let mut command = std::process::Command::new("zig");
        apply_include_paths(
            &mut command,
            &[
                std::path::PathBuf::from("include"),
                std::path::PathBuf::from("vendor/include"),
            ],
        );

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-I", "include", "-I", "vendor/include"]
        );
    }
}
