use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidation, SnippetValidator, run_command};
use std::io::Write;

pub struct TypeScriptValidator;

const SNIPPET_FILE_NAME: &str = "snippet.ts";
const BATCH_FILE_PREFIX: &str = "snippet_batch_";

/// A file whose top level contains an `export` is a module, and a module's top-level names are
/// scoped to it. Without this every batched file shares one global script scope, so two snippets
/// each declaring `const result` collide with TS2451 — a failure the one-process-per-snippet path
/// could never produce. ~keep
const MODULE_SCOPE_MARKER: &str = "export {};";

const UNRESOLVED_BATCH_SLOT: &str = "TypeScript batch validation produced no result for this snippet";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "tsc failed without a snippet-specific diagnostic";

impl TypeScriptValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let mut results: Vec<Option<SnippetValidation>> = vec![None; snippets.len()];
        let mut checked = Vec::new();
        for (index, snippet) in snippets.iter().enumerate() {
            if Self::is_trivially_valid(&snippet.code) {
                results[index] = Some((SnippetStatus::Pass, None));
            } else {
                checked.push(index);
            }
        }
        if !checked.is_empty() {
            let file_names = checked
                .iter()
                .map(|index| format!("{BATCH_FILE_PREFIX}{index}.ts"))
                .collect::<Vec<_>>();
            let outcomes = Self::check_batch(snippets, &checked, &file_names, level, timeout_secs, session)?;
            for (index, outcome) in checked.into_iter().zip(outcomes) {
                results[index] = Some(outcome);
            }
        }
        Ok(results
            .into_iter()
            .map(|value| value.unwrap_or_else(|| (SnippetStatus::Error, Some(UNRESOLVED_BATCH_SLOT.to_string()))))
            .collect())
    }

    fn check_batch(
        snippets: &[&Snippet],
        checked: &[usize],
        file_names: &[String],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let temporary_directory = session.is_none().then(ScratchDir::isolated).transpose()?;
        let directory = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.workspace_directory()?,
            (None, Some(value)) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        if session.is_none() {
            std::fs::write(directory.join("tsconfig.json"), Self::isolated_tsconfig())?;
        }
        for (index, file_name) in checked.iter().zip(file_names) {
            let code = Self::as_module(&Self::dedent(&snippets[*index].code));
            std::fs::write(directory.join(file_name), code)?;
        }
        let project = session
            .and_then(|value| value.manifest.as_ref())
            .map(|manifest| Self::write_overlay_config_for(&directory, manifest, file_names))
            .transpose()?;
        let mut command = Self::check_command(level, &directory, project.as_deref());
        if let Some(session) = session {
            session.apply(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(file_names, success, &output))
    }

    /// Attributes `tsc --pretty false` diagnostics — `path/to/file.ts(line,col): error TSxxxx: …` —
    /// back to the snippet that owns each file. Continuation lines of a message chain carry no path
    /// of their own, so they stay with the diagnostic they follow. ~keep
    fn batch_results(file_names: &[String], success: bool, output: &str) -> BatchValidation {
        let mut diagnostics = vec![Vec::new(); file_names.len()];
        let mut unmatched = Vec::new();
        let mut current: Option<usize> = None;
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match Self::diagnostic_owner(file_names, line) {
                Some(index) => {
                    diagnostics[index].push(line.to_string());
                    current = Some(index);
                }
                None if line.starts_with([' ', '\t']) => match current {
                    Some(index) => diagnostics[index].push(line.to_string()),
                    None => unmatched.push(line.to_string()),
                },
                None => {
                    current = None;
                    unmatched.push(line.to_string());
                }
            }
        }
        let attributed = diagnostics.iter().any(|messages| !messages.is_empty());
        let fallback = (!success && !attributed).then(|| {
            if unmatched.is_empty() {
                BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
            } else {
                unmatched.join("\n")
            }
        });
        diagnostics
            .into_iter()
            .map(|messages| match (messages.is_empty(), &fallback) {
                (true, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (true, None) => (SnippetStatus::Pass, None),
                (false, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
            })
            .collect()
    }

    fn diagnostic_owner(file_names: &[String], line: &str) -> Option<usize> {
        let (path, _) = line.split_once('(')?;
        let name = std::path::Path::new(path).file_name()?;
        file_names
            .iter()
            .position(|file_name| std::ffi::OsStr::new(file_name.as_str()) == name)
    }

    fn as_module(code: &str) -> String {
        format!("{}\n{MODULE_SCOPE_MARKER}\n", code.trim_end())
    }

    fn is_trivially_valid(code: &str) -> bool {
        Self::is_api_signature(code) || code.trim().starts_with("!!!") || code.trim().starts_with("???")
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if Self::is_trivially_valid(&snippet.code) {
            return Ok((SnippetStatus::Pass, None));
        }
        let temporary_directory = session.is_none().then(ScratchDir::isolated).transpose()?;
        let directory = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.workspace_directory()?,
            (None, Some(value)) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        if session.is_none() {
            std::fs::write(directory.join("tsconfig.json"), Self::isolated_tsconfig())?;
        }
        let file_path = directory.join(SNIPPET_FILE_NAME);
        let mut file = std::fs::File::create(&file_path)?;
        file.write_all(Self::dedent(&snippet.code).as_bytes())?;
        let project = session
            .and_then(|value| value.manifest.as_ref())
            .map(|manifest| Self::write_overlay_config(&directory, manifest))
            .transpose()?;
        let mut command = Self::command(level, &file_path, &directory, session, project.as_deref());
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn isolated_tsconfig() -> &'static str {
        r#"{"compilerOptions":{"strict":true,"noEmit":true,"target":"ES2022","module":"ES2022","moduleResolution":"bundler","skipLibCheck":true},"include":["*.ts"]}"#
    }

    fn write_overlay_config(directory: &std::path::Path, manifest: &std::path::Path) -> Result<std::path::PathBuf> {
        Self::write_overlay_config_for(directory, manifest, &[SNIPPET_FILE_NAME.to_string()])
    }

    fn write_overlay_config_for(
        directory: &std::path::Path,
        manifest: &std::path::Path,
        file_names: &[String],
    ) -> Result<std::path::PathBuf> {
        let path = directory.join("tsconfig.json");
        let manifest_value: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest)?).map_err(|error| {
            crate::snippets::error::Error::Other(format!(
                "parsing TypeScript package manifest {}: {error}",
                manifest.display()
            ))
        })?;
        let content = if manifest_value.get("compilerOptions").is_some() {
            Self::project_overlay(directory, manifest, file_names)
        } else {
            Self::package_overlay(manifest, &manifest_value, file_names)?
        };
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&content).map_err(|error| {
                crate::snippets::error::Error::Other(format!("serializing TypeScript snippet config: {error}"))
            })?,
        )?;
        Ok(path)
    }

    fn project_overlay(
        directory: &std::path::Path,
        manifest: &std::path::Path,
        file_names: &[String],
    ) -> serde_json::Value {
        let files = file_names.iter().map(|name| directory.join(name)).collect::<Vec<_>>();
        serde_json::json!({
            "extends": manifest,
            "compilerOptions": { "noEmit": true },
            "files": files
        })
    }

    fn package_overlay(
        manifest: &std::path::Path,
        manifest_value: &serde_json::Value,
        file_names: &[String],
    ) -> Result<serde_json::Value> {
        let package_name = manifest_value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                crate::snippets::error::Error::Other(format!("no package name in {}", manifest.display()))
            })?;
        let package_root = manifest.parent().unwrap_or_else(|| std::path::Path::new("."));
        let declaration = manifest_value
            .get("types")
            .or_else(|| manifest_value.get("typings"))
            .and_then(serde_json::Value::as_str)
            .map(|entry| package_root.join(entry))
            .unwrap_or_else(|| package_root.to_path_buf());
        Ok(serde_json::json!({
            "compilerOptions": {
                "strict": true,
                "noEmit": true,
                "target": "ES2022",
                "module": "ES2022",
                "moduleResolution": "bundler",
                "skipLibCheck": true,
                "paths": { package_name: [declaration] }
            },
            "files": file_names
        }))
    }

    fn command(
        level: ValidationLevel,
        file_path: &std::path::Path,
        isolated_directory: &std::path::Path,
        session: Option<&ValidationSession>,
        project: Option<&std::path::Path>,
    ) -> std::process::Command {
        let mut command = if level == ValidationLevel::Run {
            let mut command = std::process::Command::new("tsx");
            if let Some(project) = project {
                command.args(["--tsconfig", project.to_string_lossy().as_ref()]);
            }
            command.arg(file_path);
            command
        } else {
            Self::check_command(level, isolated_directory, project)
        };
        if let Some(session) = session {
            session.apply(&mut command);
        }
        command
    }

    fn check_command(
        level: ValidationLevel,
        isolated_directory: &std::path::Path,
        project: Option<&std::path::Path>,
    ) -> std::process::Command {
        let mut command = std::process::Command::new("tsc");
        command.args(["--noEmit", "--pretty", "false"]);
        if level == ValidationLevel::Syntax {
            command.arg("--noCheck");
        }
        if let Some(project) = project {
            command.args(["--project", project.to_string_lossy().as_ref()]);
        } else {
            command.current_dir(isolated_directory);
        }
        command
    }

    fn dedent(code: &str) -> String {
        let min_indent = code
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.len() - line.trim_start().len())
            .min()
            .unwrap_or(0);

        if min_indent == 0 {
            return code.to_string();
        }

        code.lines()
            .map(|line| {
                if line.trim().is_empty() {
                    String::new()
                } else if line.len() > min_indent {
                    line[min_indent..].to_string()
                } else {
                    line.trim().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn is_api_signature(code: &str) -> bool {
        let trimmed = code.trim();

        if trimmed.lines().count() <= 6 {
            let has_fn_decl = trimmed.starts_with("function ")
                || trimmed.starts_with("async function ")
                || trimmed.starts_with("export function ")
                || trimmed.starts_with("export async function ");
            return has_fn_decl && !trimmed.contains('{');
        }

        false
    }
}

impl SnippetValidator for TypeScriptValidator {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn is_available(&self) -> bool {
        which::which("tsc").is_ok()
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

    /// `Run` is declined: `tsx` executes one file and its stdout belongs to that snippet alone, so
    /// there is nothing to attribute back across a batch. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        (level != ValidationLevel::Run)
            .then(|| Self::validate_batch_with_context(snippets, level, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        let patterns = [
            "TS2307", "TS2304", "TS2305", "TS2306", "TS2322", "TS2345", "TS2339", "TS2351", "TS2552", "TS2314",
            "TS2391", "TS2693", "TS7016", "TS2371", "TS2580", "TS1375", "TS2792", "TS2503", "TS7006", "TS2769",
            "TS1128", "TS1005", "TS18046", "TS18047", "TS2531", "TS2532", "TS2451",
        ];

        let error_lines: Vec<&str> = output.lines().filter(|line| line.contains("error TS")).collect();
        if error_lines.is_empty() {
            return false;
        }

        error_lines
            .iter()
            .all(|line| patterns.iter().any(|pattern| line.contains(pattern)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    #[test]
    fn package_manifest_maps_local_declarations() {
        let package = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let manifest = package.path().join("package.json");
        std::fs::write(&manifest, r#"{"name":"sample-binding","types":"index.d.ts"}"#).unwrap();
        let config = TypeScriptValidator::write_overlay_config(scratch.path(), &manifest).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
        assert_eq!(
            value["compilerOptions"]["paths"]["sample-binding"][0],
            package.path().join("index.d.ts").to_string_lossy().as_ref()
        );
        assert!(value["compilerOptions"].get("baseUrl").is_none());
    }

    #[test]
    fn project_manifest_resolves_declared_local_package_and_replaces_stale_source() {
        if which::which("tsc").is_err() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let package = project.path().join("node_modules/sample-binding");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"sample-binding","types":"index.d.ts"}"#,
        )
        .unwrap();
        std::fs::write(package.join("index.d.ts"), "export declare const value: number;\n").unwrap();
        let manifest = project.path().join("tsconfig.json");
        std::fs::write(
            &manifest,
            r#"{"compilerOptions":{"strict":true,"moduleResolution":"bundler","module":"ES2022"}}"#,
        )
        .unwrap();
        let session = ValidationSession {
            language: Language::TypeScript,
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let valid = snippet("import { value } from 'sample-binding';\nconst result: number = value;");
        let invalid = snippet("import { value } from 'sample-binding';\nconst result: string = value;");
        let (first, _) =
            TypeScriptValidator::validate_with_context(&valid, ValidationLevel::TypeCheck, 30, Some(&session)).unwrap();
        let (second, _) =
            TypeScriptValidator::validate_with_context(&invalid, ValidationLevel::TypeCheck, 30, Some(&session))
                .unwrap();

        assert_eq!(first, SnippetStatus::Pass);
        assert_eq!(second, SnippetStatus::Fail);
    }

    /// The batch overlay must carry the session manifest's own compiler options, or every batched
    /// snippet fails on an unresolved import of the generated bindings instead of on its own code. ~keep
    #[test]
    fn batch_in_a_session_resolves_the_local_package_the_manifest_declares() {
        if which::which("tsc").is_err() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let package = project.path().join("node_modules/sample-binding");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"sample-binding","types":"index.d.ts"}"#,
        )
        .unwrap();
        std::fs::write(package.join("index.d.ts"), "export declare const value: number;\n").unwrap();
        let manifest = project.path().join("tsconfig.json");
        std::fs::write(
            &manifest,
            r#"{"compilerOptions":{"strict":true,"moduleResolution":"bundler","module":"ES2022"}}"#,
        )
        .unwrap();
        let session = ValidationSession {
            language: Language::TypeScript,
            working_directory: project.path().to_path_buf(),
            manifest: Some(manifest),
            fingerprint: "neutral-project".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let valid =
            snippet("import { value } from 'sample-binding';\nconst result: number = value;\nconsole.log(result);");
        let invalid =
            snippet("import { value } from 'sample-binding';\nconst result: string = value;\nconsole.log(result);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&valid, &invalid, &valid],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            Some(&session),
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    #[test]
    fn batch_declines_run_because_each_snippet_owns_its_own_output() {
        let only = snippet("console.log(1);");

        let declined = TypeScriptValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 30, None);

        assert!(declined.is_none());
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if which::which("tsc").is_err() {
            return;
        }
        let first = snippet("const first: number = 1;\nconsole.log(first);");
        let second = snippet("const second: string = 'two';\nconsole.log(second);");
        let third = snippet("const third: boolean = true;\nconsole.log(third);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&first, &second, &third],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[1], (SnippetStatus::Pass, None));
        assert_eq!(results[2], (SnippetStatus::Pass, None));
    }

    #[test]
    fn batch_fails_only_the_broken_snippet_and_passes_its_neighbours() {
        if which::which("tsc").is_err() {
            return;
        }
        let first = snippet("const value: number = 1;\nconsole.log(value);");
        let broken = snippet("const value: string = 2;\nconsole.log(value);");
        let third = snippet("const value: boolean = true;\nconsole.log(value);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::TypeCheck,
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
                .is_some_and(|message| message.contains("TS2322")),
            "the failing snippet must carry its own diagnostic: {:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// Every batched file lands in one tsc project, where top-level `const` declarations share a
    /// global script scope unless each file is a module. Two snippets both naming `result` would
    /// then both fail with TS2451 — a failure neither has when validated alone. ~keep
    #[test]
    fn batch_does_not_invent_redeclaration_failures_for_snippets_sharing_a_name() {
        if which::which("tsc").is_err() {
            return;
        }
        let first = snippet("const result: number = 1;\nconsole.log(result);");
        let second = snippet("const result: number = 2;\nconsole.log(result);");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&first, &second],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    #[test]
    fn batch_passes_signature_only_snippets_without_compiling_them() {
        let signature = snippet("export function build(name: string): Promise<number>");
        let placeholder = snippet("!!! note\n    see the guide");

        let results = TypeScriptValidator::validate_batch_with_context(
            &[&signature, &placeholder],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    #[test]
    fn diagnostics_attach_to_the_file_named_at_the_start_of_the_line() {
        let file_names = vec!["snippet_batch_0.ts".to_string(), "snippet_batch_1.ts".to_string()];
        let output = "snippet_batch_1.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.\n";

        let results = TypeScriptValidator::batch_results(&file_names, false, output);

        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert_eq!(
            results[1].1.as_deref(),
            Some("snippet_batch_1.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.")
        );
    }

    /// A compiler that fails for a reason no file owns — a bad tsconfig, a missing library — must
    /// fail every snippet carrying the real output, never silently pass them all. ~keep
    #[test]
    fn a_project_wide_failure_fails_every_snippet_with_the_real_output() {
        let file_names = vec!["snippet_batch_0.ts".to_string(), "snippet_batch_1.ts".to_string()];
        let output = "error TS5083: Cannot read file 'tsconfig.json'.\n";

        let results = TypeScriptValidator::batch_results(&file_names, false, output);

        assert_eq!(results.len(), 2);
        for result in &results {
            assert_eq!(result.0, SnippetStatus::Fail);
            assert_eq!(
                result.1.as_deref(),
                Some("error TS5083: Cannot read file 'tsconfig.json'.")
            );
        }
    }

    #[test]
    fn message_chain_continuation_lines_stay_with_their_diagnostic() {
        let file_names = vec!["snippet_batch_0.ts".to_string(), "snippet_batch_1.ts".to_string()];
        let output = "snippet_batch_0.ts(2,1): error TS2345: Argument mismatch.\n  Types of property 'id' differ.\n";

        let results = TypeScriptValidator::batch_results(&file_names, false, output);

        assert_eq!(
            results[0].1.as_deref(),
            Some("snippet_batch_0.ts(2,1): error TS2345: Argument mismatch.\n  Types of property 'id' differ.")
        );
        assert_eq!(results[1], (SnippetStatus::Pass, None));
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: "snippet.ts".into(),
            language: Language::TypeScript,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "snippet.ts".into(),
                line: 1,
                block_index: 0,
            },
        }
    }
}
