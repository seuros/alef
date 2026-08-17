use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use std::io::Write;

pub struct TypeScriptValidator;

impl TypeScriptValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if Self::is_api_signature(&snippet.code)
            || snippet.code.trim().starts_with("!!!")
            || snippet.code.trim().starts_with("???")
        {
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
        let file_path = directory.join("snippet.ts");
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
        let path = directory.join("tsconfig.json");
        let manifest_value: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest)?).map_err(|error| {
            crate::snippets::error::Error::Other(format!(
                "parsing TypeScript package manifest {}: {error}",
                manifest.display()
            ))
        })?;
        let content = if manifest_value.get("compilerOptions").is_some() {
            Self::project_overlay(directory, manifest)
        } else {
            Self::package_overlay(manifest, &manifest_value)?
        };
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&content).map_err(|error| {
                crate::snippets::error::Error::Other(format!("serializing TypeScript snippet config: {error}"))
            })?,
        )?;
        Ok(path)
    }

    fn project_overlay(directory: &std::path::Path, manifest: &std::path::Path) -> serde_json::Value {
        serde_json::json!({
            "extends": manifest,
            "compilerOptions": { "noEmit": true },
            "files": [directory.join("snippet.ts")]
        })
    }

    fn package_overlay(manifest: &std::path::Path, manifest_value: &serde_json::Value) -> Result<serde_json::Value> {
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
            "files": ["snippet.ts"]
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
        };
        if let Some(session) = session {
            session.apply(&mut command);
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
