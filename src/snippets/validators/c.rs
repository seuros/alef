use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};
use std::io::Write;
use tempfile::NamedTempFile;

pub struct CValidator;

const NO_C_COMPILER: &str = "no C compiler on PATH";
const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the C compiler failed without a snippet-specific diagnostic";

/// The substring that separates a diagnostic the compiler rejects the translation unit over from
/// one it merely reports. Every snippet is compiled with the same flags the per-snippet path uses,
/// so a warning that leaves that path passing must leave the batch passing too — attributing a
/// line to a snippet is not the same as failing it. `fatal error:` ends with this marker as well. ~keep
const ERROR_DIAGNOSTIC_MARKER: &str = "error:";

fn compiler() -> Option<String> {
    for candidate in ["cc", "clang", "gcc"] {
        if which::which(candidate).is_ok() {
            return Some(candidate.to_string());
        }
    }
    None
}

impl CValidator {
    /// One compiler start for the whole batch instead of one per snippet.
    ///
    /// `Syntax`/`TypeCheck` reach for `-fsyntax-only`, which accepts many sources and type-checks
    /// each as its own translation unit, so the `main` every snippet declares never collides —
    /// nothing is linked. `Compile` reaches for `-c` instead: given N sources with no `-o`, `cc`
    /// compiles each into its own object file named after that source's stem, entirely
    /// independently of the others — no shared output, no link step, so the same one-invocation
    /// shape covers it too. `Run` is the one level this cannot serve: it needs a linked, runnable
    /// executable *per snippet*, and there is no flag that produces N distinct executables from one
    /// invocation the way `-c`'s implicit per-source naming does for object files — that one still
    /// declines in `validate_batch_in_session` and falls back to one process per snippet. ~keep
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let Some(cc) = compiler() else {
            return Ok(vec![
                (SnippetStatus::Unavailable, Some(NO_C_COMPILER.into()));
                snippets.len()
            ]);
        };
        let dir = match session {
            Some(session) => session.scratch_dir()?,
            None => ScratchDir::isolated()?,
        };
        let mut file_names = Vec::with_capacity(snippets.len());
        let mut paths = Vec::with_capacity(snippets.len());
        for (index, snippet) in snippets.iter().enumerate() {
            let file_name = format!("{BATCH_FILE_PREFIX}{index}.c");
            let path = dir.path().join(&file_name);
            std::fs::write(&path, snippet.code.as_bytes())?;
            file_names.push(file_name);
            paths.push(path);
        }
        let mut command = std::process::Command::new(cc);
        if level == ValidationLevel::Compile {
            command.arg("-c");
        } else {
            command.arg("-fsyntax-only");
            if level == ValidationLevel::TypeCheck {
                command.args(["-Wall", "-Werror"]);
            }
        }
        if let Some(session) = session {
            apply_session_includes(&mut command, session);
        }
        command.args(&paths);
        if let Some(session) = session {
            session.apply(&mut command);
        }
        // A batched `-c` compile writes one `.o` per source into the compiler's current
        // directory — there is no per-file `-o` for a multi-source invocation, so the implicit
        // destination is all a batch call has. `session.apply` above points the process at the
        // session's `working_directory` (needed for env resolution and any relative toolchain-cache
        // override), which would otherwise scatter these object files into the real project tree
        // this run is validating bindings against. Overriding `current_dir` to the batch's own
        // fresh, self-removing scratch directory afterwards confines them there instead — the two
        // `Command` methods are independent knobs, so this does not undo `session.apply`'s env. ~keep
        if level == ValidationLevel::Compile {
            command.current_dir(dir.path());
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(&file_names, success, &output))
    }

    /// Attributes compiler output back to the snippet that owns it. Every diagnostic opens with
    /// its own source path (`snippet_batch_2.c:4:9: error: …`), and the caret/source lines that
    /// follow carry no path at all, so a pathless line stays with the file last named. ~keep
    fn batch_results(file_names: &[String], success: bool, output: &str) -> BatchValidation {
        let mut diagnostics = vec![Vec::new(); file_names.len()];
        let mut rejected = vec![false; file_names.len()];
        let mut unmatched = Vec::new();
        let mut current = None;
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match Self::file_owner(file_names, line).or(current) {
                Some(index) => {
                    current = Some(index);
                    diagnostics[index].push(line.to_string());
                    rejected[index] |= line.contains(ERROR_DIAGNOSTIC_MARKER);
                }
                None => unmatched.push(line.to_string()),
            }
        }
        let attributed = rejected.iter().any(|value| *value);
        let fallback = (!success && !attributed).then(|| {
            if unmatched.is_empty() {
                BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
            } else {
                unmatched.join("\n")
            }
        });
        rejected
            .into_iter()
            .zip(diagnostics)
            .map(|(rejected, messages)| match (rejected, &fallback) {
                (true, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
                (false, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (false, None) => (SnippetStatus::Pass, None),
            })
            .collect()
    }

    fn file_owner(file_names: &[String], line: &str) -> Option<usize> {
        file_names
            .iter()
            .position(|file_name| line.contains(file_name.as_str()))
    }
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
            return Ok((SnippetStatus::Unavailable, Some(NO_C_COMPILER.into())));
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
            return Ok((SnippetStatus::Unavailable, Some(NO_C_COMPILER.into())));
        };
        let scratch_dir = session.scratch_dir()?;
        let mut source = tempfile::Builder::new().suffix(".c").tempfile_in(scratch_dir.path())?;
        source.write_all(snippet.code.as_bytes())?;
        source.flush()?;
        let output = scratch_dir.path().join(".alef-snippet-output");
        let mut command = std::process::Command::new(cc);
        apply_session_includes(&mut command, session);
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

    /// `Run` is the only level that declines: it needs a linked, runnable executable per snippet,
    /// and unlike `-c`'s implicit per-source object naming, there is no way to get N distinct
    /// executables from one invocation. `Syntax`, `TypeCheck` and `Compile` all batch — see
    /// `validate_batch_with_context` for how each shapes its one compiler invocation. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        matches!(
            level,
            ValidationLevel::Syntax | ValidationLevel::TypeCheck | ValidationLevel::Compile
        )
        .then(|| Self::validate_batch_with_context(snippets, level, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
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

fn apply_session_includes(command: &mut std::process::Command, session: &ValidationSession) {
    let include_directory = session
        .manifest
        .as_deref()
        .and_then(std::path::Path::parent)
        .unwrap_or(&session.working_directory);
    command.arg("-I").arg(include_directory);
    apply_include_paths(command, &session.include_paths);
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

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn batch_declines_only_the_level_that_links_an_executable() {
        let only = snippet("int main(void) { return 0; }\n");

        let declined = CValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 10, None);
        assert!(declined.is_none(), "Run must fall back to one process per snippet");
    }

    /// The claim this pins: `Compile` does not need its own linked artifact the way `Run` does, so
    /// it batches through `-c` exactly like `Syntax`/`TypeCheck` batch through `-fsyntax-only`.
    #[test]
    fn batch_accepts_compile_level() {
        let only = snippet("int main(void) { return 0; }\n");

        let accepted = CValidator.validate_batch_in_session(&[&only], ValidationLevel::Compile, 10, None);
        assert!(
            accepted.is_some(),
            "Compile must batch: one `-c` invocation covers every source"
        );
    }

    /// Real-toolchain proof that a batched `Compile` invocation produces one object file per
    /// source with no shared `-o` and no collision between two snippets that each declare `main` --
    /// unlike `Run`, `-c` never links, so the duplicate-symbol failure a linked batch would hit
    /// never applies here. ~keep
    #[test]
    fn compile_batch_passes_two_snippets_that_each_declare_main() {
        if compiler().is_none() {
            return;
        }
        let first = snippet("int main(void) { return 0; }\n");
        let second = snippet("int main(void) { return 1; }\n");

        let results = CValidator::validate_batch_with_context(
            &[&first, &second],
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// The compile-level counterpart of `batch_fails_only_the_broken_snippet_and_passes_its_neighbours`:
    /// a broken snippet's `-c` diagnostic must still attribute to only that snippet, not fail the
    /// whole batch.
    #[test]
    fn compile_batch_fails_only_the_broken_snippet_and_passes_its_neighbours() {
        if compiler().is_none() {
            return;
        }
        let first = snippet("int first(void) { return 1; }\n");
        let broken = snippet("int second(void) { @@@ }\n");
        let third = snippet("int third(void) { return 3; }\n");

        let results = CValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// The object files a batched `Compile` produces must land in the batch's own scratch
    /// directory, not wherever the process's working directory happens to be -- otherwise a real
    /// session run would scatter `.o` files into the consumer's project tree. `ScratchDir::isolated`
    /// (the `session: None` path) creates and returns its own temp directory, so this asserts
    /// against *that* directory rather than the test binary's `current_dir`.
    #[test]
    fn compile_batch_writes_object_files_into_its_own_scratch_directory_not_the_process_cwd() {
        if compiler().is_none() {
            return;
        }
        let only = snippet("int main(void) { return 0; }\n");

        let results = CValidator::validate_batch_with_context(
            &[&only],
            ValidationLevel::Compile,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None)]);
        let leaked_in_cwd = std::env::current_dir()
            .expect("current dir")
            .join(format!("{BATCH_FILE_PREFIX}0.o"));
        assert!(
            !leaked_in_cwd.exists(),
            "a batched compile must not write its object file into the process's own working \
             directory: {}",
            leaked_in_cwd.display()
        );
        let _ = std::fs::remove_file(leaked_in_cwd);
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if compiler().is_none() {
            return;
        }
        let first = snippet("int first(void) { return 1; }\n");
        let second = snippet("int second(void) { return 2; }\n");
        let third = snippet("int third(void) { return 3; }\n");

        let results = CValidator::validate_batch_with_context(
            &[&first, &second, &third],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None)
            ]
        );
    }

    #[test]
    fn batch_fails_only_the_broken_snippet_and_passes_its_neighbours() {
        if compiler().is_none() {
            return;
        }
        let first = snippet("int first(void) { return 1; }\n");
        let broken = snippet("int second(void) { @@@ }\n");
        let third = snippet("int third(void) { return 3; }\n");

        let results = CValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("error:")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// The isolation the `-fsyntax-only` level buys: every snippet declares its own `main`, which
    /// would be a duplicate-symbol link failure the moment the batch linked them together — and a
    /// failure the per-snippet path never produces. ~keep
    #[test]
    fn batch_passes_two_snippets_that_each_declare_main() {
        if compiler().is_none() {
            return;
        }
        let first = snippet("int main(void) { return 0; }\n");
        let second = snippet("int main(void) { return 1; }\n");

        let results = CValidator::validate_batch_with_context(
            &[&first, &second],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// Attributing a compiler line to a snippet is not the same as failing it: `-fsyntax-only`
    /// without `-Werror` reports warnings and still exits 0, exactly as the per-snippet path does,
    /// so a warned-about snippet must keep passing. ~keep
    #[test]
    fn batch_does_not_fail_a_snippet_the_compiler_only_warns_about() {
        if compiler().is_none() {
            return;
        }
        let first = snippet("int first(void) { return 1; }\n");
        let warned = snippet("#warning batch fixture warning\nint second(void) { return 2; }\n");

        let results = CValidator::validate_batch_with_context(
            &[&first, &warned],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// A compiler that fails without naming any snippet must not let the batch pass: every snippet
    /// carries the real output instead. ~keep
    #[test]
    fn batch_results_fail_every_snippet_when_no_diagnostic_names_one() {
        let file_names = vec!["snippet_batch_0.c".to_string(), "snippet_batch_1.c".to_string()];

        let results = CValidator::batch_results(&file_names, false, "cc: error: unrecognized command-line option\n");

        assert_eq!(
            results,
            vec![
                (
                    SnippetStatus::Fail,
                    Some("cc: error: unrecognized command-line option".to_string())
                ),
                (
                    SnippetStatus::Fail,
                    Some("cc: error: unrecognized command-line option".to_string())
                ),
            ]
        );
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
