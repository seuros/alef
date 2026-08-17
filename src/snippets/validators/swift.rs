use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};

pub struct SwiftValidator;

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
        let module_directories = swift_module_directories(session)?;
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

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("no such module") || output.contains("cannot find") && output.contains("in scope")
    }
}

fn swift_module_directories(session: &ValidationSession) -> Result<Vec<std::path::PathBuf>> {
    let mut command = std::process::Command::new("swift");
    command.args(["build", "--show-bin-path"]);
    session.apply(&mut command);
    if let Some(package_root) = session.manifest.as_deref().and_then(std::path::Path::parent) {
        command.current_dir(package_root);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(crate::snippets::error::Error::Other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let binary_directory = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
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
