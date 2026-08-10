use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct CsharpValidator;

impl CsharpValidator {
    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let temporary_directory = session.is_none().then(TempDir::new).transpose()?;
        let directory = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.workspace_directory()?,
            (None, Some(value)) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        let project_path = directory.join("Snippet.csproj");
        let target_framework = session
            .and_then(|value| value.manifest.as_deref())
            .map(Self::target_framework)
            .transpose()?
            .unwrap_or_else(|| "net8.0".to_owned());
        let reference = session
            .and_then(|value| value.manifest.as_ref())
            .map_or_else(String::new, |manifest| {
                format!(
                    "  <ItemGroup><ProjectReference Include={:?} /></ItemGroup>\n",
                    manifest.to_string_lossy()
                )
            });
        let project = format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>{target_framework}</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings></PropertyGroup>{reference}</Project>\n"
        );
        std::fs::write(&project_path, project)?;
        std::fs::write(directory.join("Program.cs"), Self::wrap_if_fragment(&snippet.code))?;
        let mut command = std::process::Command::new("dotnet");
        match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => {
                command.args(["build", "--nologo", "-v", "quiet"]);
            }
            ValidationLevel::TypeCheck => {
                command.args(["build", "--nologo", "-v", "quiet", "-warnaserror"]);
            }
            ValidationLevel::Run => {
                command.args(["run", "--nologo"]);
            }
        }
        command.current_dir(&directory);
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

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        let only_comments = !trimmed.is_empty()
            && trimmed
                .lines()
                .all(|line| line.trim().is_empty() || line.trim().starts_with("//"));
        if only_comments {
            return format!("{trimmed}\n// snippet placeholder\nreturn;\n");
        }
        code.to_string()
    }

    fn target_framework(manifest: &std::path::Path) -> Result<String> {
        let source = std::fs::read_to_string(manifest)?;
        for element in ["TargetFramework", "TargetFrameworks"] {
            let opening = format!("<{element}>");
            let closing = format!("</{element}>");
            if let Some(start) = source.find(&opening)
                && let Some(end) = source[start + opening.len()..].find(&closing)
            {
                let value = &source[start + opening.len()..start + opening.len() + end];
                if let Some(framework) = value.split(';').map(str::trim).find(|value| !value.is_empty()) {
                    return Ok(framework.to_owned());
                }
            }
        }
        Err(crate::snippets::error::Error::Other(format!(
            "no target framework in {}",
            manifest.display()
        )))
    }

    fn is_dependency_error_text(output: &str) -> bool {
        output.contains("CS0246") || output.contains("CS0234") || output.contains("CS0103") || output.contains("CS5001")
    }
}

impl SnippetValidator for CsharpValidator {
    fn language(&self) -> Language {
        Language::Csharp
    }

    fn is_available(&self) -> bool {
        which::which("dotnet").is_ok()
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
        Self::is_dependency_error_text(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn session_manifest_adds_a_real_project_reference() {
        if which::which("dotnet").is_err() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let project = root.path().join("LocalFixture");
        let working = root.path().join("working");
        std::fs::create_dir_all(&project).expect("project directory");
        std::fs::create_dir_all(&working).expect("working directory");
        std::fs::write(
            project.join("LocalFixture.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>",
        )
        .expect("project manifest");
        std::fs::write(
            project.join("Value.cs"),
            "namespace LocalFixture; public static class Values { public const int Value = 1; }",
        )
        .expect("project source");
        let session = ValidationSession {
            working_directory: working,
            manifest: Some(project.join("LocalFixture.csproj")),
            fingerprint: "fixture".into(),
            env: BTreeMap::from([("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into())]),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = CsharpValidator::validate_with_context(
            &snippet("using LocalFixture; System.Console.WriteLine(Values.Value);"),
            ValidationLevel::TypeCheck,
            60,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    #[test]
    fn derives_target_framework_from_referenced_project() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let manifest = directory.path().join("Fixture.csproj");
        std::fs::write(
            &manifest,
            "<Project><PropertyGroup><TargetFrameworks>net10.0;net9.0</TargetFrameworks></PropertyGroup></Project>",
        )
        .expect("project manifest");

        assert_eq!(CsharpValidator::target_framework(&manifest).unwrap(), "net10.0");
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.cs"),
            language: Language::Csharp,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.cs"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
