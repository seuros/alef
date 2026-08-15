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
        let mut declared_include_paths = Vec::new();
        if level == ValidationLevel::Syntax {
            command.arg(&file);
        } else if let Some(manifest) = session.manifest.as_deref() {
            let (module_name, module_source) = zig_package_module(manifest)?;
            command
                .args(["--dep", &module_name])
                .arg(format!("-Mroot={}", file.display()))
                .arg(format!("-M{module_name}={}", module_source.display()));
            declared_include_paths = zig_manifest_include_paths(manifest)?
                .into_iter()
                .map(|path| session.working_directory.join(path))
                .collect();
        } else {
            command.arg(&file);
        }
        apply_include_paths(&mut command, &session.include_paths);
        apply_include_paths(&mut command, &declared_include_paths);
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

/// Include directories the build manifest declares for its module, in declaration order. ~keep
///
/// A `build.zig` is a program rather than a manifest, so this reads back only the shape Alef's own
/// `build_zig.jinja` emits: `addIncludePath(.{ .cwd_relative = <expr> })`, where `<expr>` is either
/// a string literal or an identifier bound by `const <expr> = b.option(...) orelse "<default>";`.
/// Any other expression is skipped rather than guessed at — a wrong `-I` is worse than none.
///
/// Without this the reconstructed `build-exe` command carries no `-I` at all unless the consumer
/// also repeats the path under `include_paths`, so every snippet reaching a `@cInclude` in the
/// binding fails with `C import failed ... 'header.h' not found` while `zig build` succeeds.
/// Paths are returned verbatim: `.cwd_relative` is relative to the build's working directory,
/// which is exactly the directory the session runs zig in.
fn zig_manifest_include_paths(manifest: &std::path::Path) -> Result<Vec<String>> {
    const DECLARATION: &str = "addIncludePath(.{ .cwd_relative = ";

    let source = std::fs::read_to_string(manifest)?;
    let mut paths: Vec<String> = Vec::new();
    for occurrence in source.split(DECLARATION).skip(1) {
        let Some(end) = occurrence.find(" })") else {
            continue;
        };
        let expression = occurrence[..end].trim();
        let Some(path) = string_literal(expression)
            .map(str::to_owned)
            .or_else(|| option_default(&source, expression))
        else {
            continue;
        };
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn string_literal(expression: &str) -> Option<&str> {
    expression.strip_prefix('"')?.strip_suffix('"')
}

/// The literal in `const <name> = b.option(...) orelse "<default>";`, if `name` is bound that way.
fn option_default(source: &str, name: &str) -> Option<String> {
    let binding = source.find(&format!("const {name} = b.option("))?;
    let default = source[binding..].find("orelse ")? + binding + "orelse ".len();
    let rest = source[default..].trim_start();
    let literal = rest.strip_prefix('"')?;
    let end = literal.find('"')?;
    Some(literal[..end].to_owned())
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

    /// Alef's own `build_zig.jinja` binds the include directory through a `b.option(...) orelse`
    /// default, so reading only string literals finds nothing in the manifest Alef itself writes.
    #[test]
    fn manifest_include_paths_resolve_through_the_build_option_default() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, sample_build_zig(true)).unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["vendor/include"]);
    }

    #[test]
    fn manifest_include_paths_accept_a_direct_string_literal() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(
            &manifest,
            "const module = b.addModule(\"sample_binding\", .{\n    .root_source_file = b.path(\"src/root.zig\"),\n});\nmodule.addIncludePath(.{ .cwd_relative = \"include\" });\n",
        )
        .unwrap();

        let paths = zig_manifest_include_paths(&manifest).unwrap();

        assert_eq!(paths, ["include"]);
    }

    #[test]
    fn a_manifest_without_an_include_declaration_contributes_no_paths() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("build.zig");
        std::fs::write(&manifest, sample_build_zig(false)).unwrap();

        assert!(zig_manifest_include_paths(&manifest).unwrap().is_empty());
    }

    /// The decisive check: a snippet whose binding module reaches a `@cInclude` compiles only when
    /// the include directory the manifest declares reaches the reconstructed `build-exe` command.
    #[test]
    fn a_snippet_compiles_against_the_include_path_its_manifest_declares() {
        if which::which("zig").is_err() {
            return;
        }

        let (declaring, declaring_session) = sample_project(true);
        let (omitting, omitting_session) = sample_project(false);
        let snippet = zig_snippet(
            "const sample_binding = @import(\"sample_binding\");\n\npub fn main() void {\n    _ = sample_binding.value();\n}\n",
        );

        let (declared_status, declared_output) = ZigValidator
            .validate_in_session(
                &snippet,
                ValidationLevel::Compile,
                TOOLCHAIN_TEST_TIMEOUT_SECS,
                Some(&declaring_session),
            )
            .expect("declaring session validates");
        let (omitted_status, _) = ZigValidator
            .validate_in_session(
                &snippet,
                ValidationLevel::Compile,
                TOOLCHAIN_TEST_TIMEOUT_SECS,
                Some(&omitting_session),
            )
            .expect("omitting session validates");

        assert_eq!(
            declared_status,
            SnippetStatus::Pass,
            "the manifest declares the include directory, so the header must resolve: {declared_output:?}"
        );
        assert_eq!(
            omitted_status,
            SnippetStatus::Fail,
            "without a declared include directory the header cannot resolve"
        );
        drop((declaring, omitting));
    }

    fn sample_build_zig(with_include: bool) -> String {
        let include = if with_include {
            "module.addIncludePath(.{ .cwd_relative = ffi_include });\n"
        } else {
            ""
        };
        format!(
            "const std = @import(\"std\");\n\
             pub fn build(b: *std.Build) void {{\n\
             \x20   const ffi_include = b.option(\n\
             \x20       []const u8,\n\
             \x20       \"ffi_include_path\",\n\
             \x20       \"Path to directory containing the FFI C header\"\n\
             \x20   ) orelse \"vendor/include\";\n\
             \x20   const module = b.addModule(\"sample_binding\", .{{\n\
             \x20       .root_source_file = b.path(\"src/root.zig\"),\n\
             \x20       .link_libc = true,\n\
             \x20   }});\n\
             \x20   {include}\
             }}\n"
        )
    }

    /// A self-contained Zig project whose module only compiles when its declared include directory
    /// is on the search path. Returns the temp dir so the caller keeps it alive.
    fn sample_project(with_include: bool) -> (tempfile::TempDir, ValidationSession) {
        let directory = tempfile::tempdir().expect("project directory");
        let root = directory.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("vendor/include")).unwrap();
        std::fs::write(root.join("build.zig"), sample_build_zig(with_include)).unwrap();
        std::fs::write(root.join("vendor/include/fixture.h"), "#define FIXTURE_VALUE 7\n").unwrap();
        std::fs::write(
            root.join("src/root.zig"),
            "pub const c = @cImport(@cInclude(\"fixture.h\"));\n\npub fn value() c_int {\n    return c.FIXTURE_VALUE;\n}\n",
        )
        .unwrap();

        let session = ValidationSession {
            working_directory: root.to_path_buf(),
            manifest: Some(root.join("build.zig")),
            fingerprint: "neutral-project".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        };
        (directory, session)
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
