use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command, run_command_streams};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SwiftModuleLookup {
    package_root: std::path::PathBuf,
    environment: Vec<(String, String)>,
}

#[derive(Default)]
pub struct SwiftValidator {
    module_directories: Mutex<HashMap<SwiftModuleLookup, std::result::Result<Vec<std::path::PathBuf>, String>>>,
}

impl SnippetValidator for SwiftValidator {
    fn language(&self) -> Language {
        Language::Swift
    }

    fn is_available(&self) -> bool {
        which::which("swiftc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = ScratchDir::isolated()?;
        let file = dir.path().join("snippet.swift");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("swiftc");
        match level {
            ValidationLevel::Syntax => {
                command.args(["-parse"]).arg(&file);
            }
            ValidationLevel::Compile => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
            }
            ValidationLevel::TypeCheck => {
                command.args(["-typecheck", "-warnings-as-errors"]).arg(&file);
            }
            ValidationLevel::Run => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
            }
        }

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::TypeCheck
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
        let dir = session.scratch_dir()?;
        let file = dir.path().join("snippet.swift");
        std::fs::write(&file, snippet.code.trim())?;
        let module_directories = self.module_directories(session, timeout_secs)?;
        let mut command = std::process::Command::new("swiftc");
        match level {
            ValidationLevel::Syntax => {
                command.arg("-parse");
            }
            ValidationLevel::TypeCheck => {
                command.args(["-typecheck", "-warnings-as-errors"]);
            }
            ValidationLevel::Compile => {
                command.arg("-typecheck");
            }
            ValidationLevel::Run => {
                command.arg("-o").arg(dir.path().join("snippet"));
            }
        }
        for directory in &module_directories {
            command.arg("-I").arg(directory);
        }
        if let Some(binary_directory) = module_directories.first().and_then(|path| path.parent()) {
            command.arg("-L").arg(binary_directory);
        }
        command.arg(&file);
        session.apply(&mut command);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    /// ~keep `cannot find 'x' in scope` fires for a name the generated snippet failed to bind
    /// just as readily as for one a missing module would have supplied, so it is the same
    /// ambiguous shape task #130 rejected for `TS2304` — and an accepted match is rewritten into
    /// `Unavailable` by `runner::finalize_result`, taking a real defect out of the failure tally.
    /// `no such module` names the unbuilt artifact itself and is unambiguous.
    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("no such module")
    }
}

impl SwiftValidator {
    fn module_directories(&self, session: &ValidationSession, timeout_secs: u64) -> Result<Vec<std::path::PathBuf>> {
        self.cached_module_directories(session, || swift_module_directories(session, timeout_secs))
    }

    fn cached_module_directories(
        &self,
        session: &ValidationSession,
        resolve: impl FnOnce() -> Result<Vec<std::path::PathBuf>>,
    ) -> Result<Vec<std::path::PathBuf>> {
        let lookup = swift_module_lookup(session);
        let mut cache = self.module_directories.lock().map_err(|error| {
            crate::snippets::error::Error::Other(format!("locking Swift module-directory cache: {error}"))
        })?;
        if let Some(cached) = cache.get(&lookup) {
            return cached.clone().map_err(crate::snippets::error::Error::Other);
        }
        // ~keep Hold the lock while SwiftPM resolves the path: releasing it here lets every
        // parallel snippet observe the same miss and launch its own `swift build --show-bin-path`.
        let resolved = resolve().map_err(|error| error.to_string());
        cache.insert(lookup, resolved.clone());
        resolved.map_err(crate::snippets::error::Error::Other)
    }
}

fn swift_module_lookup(session: &ValidationSession) -> SwiftModuleLookup {
    let package_root = session
        .manifest
        .as_deref()
        .and_then(std::path::Path::parent)
        .unwrap_or(&session.working_directory)
        .to_path_buf();
    SwiftModuleLookup {
        package_root,
        environment: session
            .env
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    }
}

/// `swift build --show-bin-path` is not a lookup: SwiftPM resolves the package first, which can
/// fetch dependencies over the network and, on a package it cannot resolve, retry until something
/// gives up. It ran unbounded here while every other subprocess in snippet validation was already
/// under the session's `timeout_secs`; it is now under the same bound, and the same process-group
/// teardown, as the `swiftc` invocation it feeds. ~keep
fn swift_module_directories(session: &ValidationSession, timeout_secs: u64) -> Result<Vec<std::path::PathBuf>> {
    let mut command = std::process::Command::new("swift");
    command.args(["build", "--show-bin-path"]);
    session.apply(&mut command);
    if let Some(package_root) = session.manifest.as_deref().and_then(std::path::Path::parent) {
        command.current_dir(package_root);
    }
    let captured = run_command_streams(&mut command, timeout_secs)?;
    if !captured.success {
        return Err(crate::snippets::error::Error::Other(
            crate::snippets::diagnostics::bounded_text(captured.stderr.trim()),
        ));
    }
    let binary_directory = std::path::PathBuf::from(captured.stdout.trim());
    swift_module_directories_in(&binary_directory)
}

fn swift_module_directories_in(binary_directory: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let mut directories = vec![binary_directory.join("Modules")];
    let Ok(entries) = std::fs::read_dir(binary_directory) else {
        return Ok(directories);
    };
    for entry in entries {
        let path = entry?.path();
        if path.join("module.modulemap").is_file() || path.join("include/module.modulemap").is_file() {
            directories.push(path);
        }
    }
    Ok(directories)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Swift is deliberately not batched. `swiftc` compiles one module per invocation, and a
    /// module accepts top-level code in exactly one file: handing it several snippets at once
    /// fails every one of them with `expressions are not allowed at the top level` before any
    /// snippet's own code is judged, which the per-snippet path never does. There is no way to
    /// scope a snippet's top-level statements into a namespace of its own, so the batch hook stays
    /// declined and the runner keeps using one process per snippet. ~keep
    #[test]
    fn batching_is_declined_because_one_module_cannot_hold_two_top_level_snippets() {
        let first = swift_snippet("print(\"one\")\n");
        let second = swift_snippet("print(\"two\")\n");

        let validator = SwiftValidator::default();
        assert!(!validator.supports_batching());
        for level in [
            ValidationLevel::Syntax,
            ValidationLevel::Compile,
            ValidationLevel::TypeCheck,
            ValidationLevel::Run,
        ] {
            let declined = validator.validate_batch_in_session(&[&first, &second], level, 10, None);
            assert!(
                declined.is_none(),
                "{level:?} must fall back to one process per snippet"
            );
        }
    }

    #[test]
    fn a_session_resolves_its_swiftpm_module_directories_once() {
        let validator = SwiftValidator::default();
        let first_session = ValidationSession {
            language: Language::Swift,
            working_directory: PathBuf::from("package"),
            manifest: Some(PathBuf::from("package/Package.swift")),
            fingerprint: "first-target-identity".into(),
            env: Default::default(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: Default::default(),
        };
        let mut second_session = first_session.clone();
        second_session.fingerprint = "second-target-identity".into();
        let invocations = AtomicUsize::new(0);
        let expected = vec![PathBuf::from(".build/debug/Modules")];

        for session in [&first_session, &second_session] {
            let resolved = validator
                .cached_module_directories(session, || {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    Ok(expected.clone())
                })
                .expect("module directories resolve");
            assert_eq!(resolved, expected);
        }

        assert_eq!(invocations.load(Ordering::SeqCst), 1);
    }

    fn swift_snippet(code: &str) -> crate::snippets::types::Snippet {
        crate::snippets::types::Snippet {
            id: None,
            path: std::path::PathBuf::from("snippet.swift"),
            language: Language::Swift,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: crate::snippets::types::SnippetMetadata::default(),
            source_origin: crate::snippets::types::SourceOrigin {
                path: std::path::PathBuf::from("snippet.swift"),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn missing_swiftpm_bin_directory_is_not_an_io_error() {
        let directory = tempfile::tempdir().expect("temp directory");
        let missing = directory.path().join("not-built");

        assert_eq!(
            swift_module_directories_in(&missing).expect("missing bin directory is tolerated"),
            vec![missing.join("Modules")]
        );
    }
}
