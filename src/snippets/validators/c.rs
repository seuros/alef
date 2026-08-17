use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::NamedTempFile;

pub struct CValidator;

fn compiler() -> Option<String> {
    for candidate in ["cc", "clang", "gcc"] {
        if which::which(candidate).is_ok() {
            return Some(candidate.to_string());
        }
    }
    None
}

impl SnippetValidator for CValidator {
    fn language(&self) -> Language {
        Language::C
    }

    fn is_available(&self) -> bool {
        compiler().is_some()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let Some(cc) = compiler() else {
            return Ok((SnippetStatus::Unavailable, Some("no C compiler on PATH".into())));
        };

        let mut source = NamedTempFile::with_suffix(".c")?;
        source.write_all(snippet.code.as_bytes())?;
        source.flush()?;
        let source_path = source.path().to_string_lossy().to_string();

        let mut command = std::process::Command::new(&cc);
        match level {
            ValidationLevel::Syntax => {
                command.args(["-fsyntax-only", &source_path]);
            }
            ValidationLevel::TypeCheck => {
                command.args(["-fsyntax-only", "-Wall", "-Werror", &source_path]);
            }
            ValidationLevel::Compile | ValidationLevel::Run => {
                // The compiled binary lives inside a guarded scratch directory rather than being
                // removed by hand at each `return`: the two `run_command` calls below both exit
                // through `?`, and neither of the old `remove_file` calls was reachable from
                // there, so a spawn failure or a timeout leaked an executable every time. ~keep
                let scratch = ScratchDir::isolated()?;
                let out_path = scratch.path().join("snippet-output").to_string_lossy().to_string();
                command.args(["-o", &out_path, &source_path]);
                let (success, output) = run_command(&mut command, timeout_secs)?;
                if !success {
                    return Ok((SnippetStatus::Fail, Some(output)));
                }
                if matches!(level, ValidationLevel::Run) {
                    let mut run = std::process::Command::new(&out_path);
                    let (ran_ok, run_output) = run_command(&mut run, timeout_secs)?;
                    return Ok(if ran_ok {
                        (SnippetStatus::Pass, None)
                    } else {
                        (SnippetStatus::Fail, Some(run_output))
                    });
                }
                return Ok((SnippetStatus::Pass, None));
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
        ValidationLevel::Run
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
        let Some(cc) = compiler() else {
            return Ok((SnippetStatus::Unavailable, Some("no C compiler on PATH".into())));
        };
        let scratch_dir = session.scratch_dir()?;
        let mut source = tempfile::Builder::new().suffix(".c").tempfile_in(scratch_dir.path())?;
        source.write_all(snippet.code.as_bytes())?;
        source.flush()?;
        let output = scratch_dir.path().join(".alef-snippet-output");
        let mut command = std::process::Command::new(cc);
        let include_directory = session
            .manifest
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or(&session.working_directory);
        command.arg("-I").arg(include_directory);
        apply_include_paths(&mut command, &session.include_paths);
        if level == ValidationLevel::Syntax {
            command.arg("-fsyntax-only");
        }
        if level == ValidationLevel::TypeCheck {
            command.args(["-fsyntax-only", "-Wall", "-Werror"]);
        }
        if level == ValidationLevel::Compile {
            command.arg("-c").arg("-o").arg(&output);
        } else if level == ValidationLevel::Run {
            command.arg("-o").arg(&output);
        }
        command.arg(source.path());
        session.apply(&mut command);
        let (success, message) = run_command(&mut command, timeout_secs)?;
        if !success {
            return Ok((SnippetStatus::Fail, Some(message)));
        }
        if level != ValidationLevel::Run {
            let _ = std::fs::remove_file(&output);
            return Ok((SnippetStatus::Pass, None));
        }
        let mut run = std::process::Command::new(&output);
        session.apply(&mut run);
        let (success, message) = run_command(&mut run, timeout_secs)?;
        let _ = std::fs::remove_file(&output);
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(message))
        })
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("file not found")
            || output.contains("No such file or directory")
            || output.contains("undeclared identifier")
            || output.contains("implicit declaration")
            || output.contains("unknown type name")
    }
}

fn apply_include_paths(command: &mut std::process::Command, include_paths: &[std::path::PathBuf]) {
    for include_path in include_paths {
        command.arg("-I").arg(include_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{Snippet, SnippetMetadata, SourceOrigin};
    use std::path::PathBuf;

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("test.c"),
            language: Language::C,
            title: None,
            code: code.to_string(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("test.c"),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn syntax_ok() {
        let v = CValidator;
        if !v.is_available() {
            return;
        }
        let s = snippet("int main(void) { return 0; }\n");
        let (status, _) = v.validate(&s, ValidationLevel::Syntax, 30).unwrap();
        assert_eq!(status, SnippetStatus::Pass);
    }

    #[test]
    fn syntax_fail() {
        let v = CValidator;
        if !v.is_available() {
            return;
        }
        let s = snippet("int main(void) { @@@ }\n");
        let (status, _) = v.validate(&s, ValidationLevel::Syntax, 30).unwrap();
        assert_eq!(status, SnippetStatus::Fail);
    }

    fn scratch_shape_session(project: &std::path::Path, fingerprint: &str) -> ValidationSession {
        ValidationSession {
            language: Language::C,
            working_directory: project.to_path_buf(),
            manifest: None,
            fingerprint: fingerprint.into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        }
    }

    fn scratch_top_level_entries(project: &std::path::Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(project)
            .expect("read project directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect()
    }

    /// Regression: `validate_in_session` used to write its source file via a bare `tempfile_in`
    /// directly against `session.working_directory`, and its compiled output to a literal
    /// `session.working_directory.join(".alef-snippet-output")` — both loose in a tracked
    /// package source directory. Both must nest under the session's own `.alef/snippets/tmp`
    /// cache root instead. ~keep
    #[test]
    fn session_scratch_resolves_under_the_cache_root_not_the_working_directory() {
        if compiler().is_none() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-shape-fixture");
        let s = snippet("int main(void) { return 0; }\n");

        let (status, output) = CValidator
            .validate_in_session(&s, ValidationLevel::Compile, 30, Some(&session))
            .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory: {leftovers:?}"
        );
    }

    /// Pins cleanup on the failure path specifically: a snippet that fails to compile must not
    /// leave its scratch source or output behind under the working directory any more than a
    /// passing one does.
    #[test]
    fn session_scratch_is_removed_after_a_run_that_fails() {
        if compiler().is_none() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-cleanup-fixture");
        let s = snippet("int main(void) { @@@ }\n");

        let (status, _) = CValidator
            .validate_in_session(&s, ValidationLevel::Compile, 30, Some(&session))
            .expect("validation runs");
        assert_eq!(status, SnippetStatus::Fail);

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory after a failing run: {leftovers:?}"
        );
        let scratch_root = project.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a failing snippet validation"
        );
    }

    #[test]
    fn session_include_paths_are_passed_to_c_compiler() {
        let mut command = std::process::Command::new("cc");
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
