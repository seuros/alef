use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// javac stops reporting after 100 errors and 100 warnings by default. Per snippet that cap is
/// harmless; across one batched invocation it is a correctness defect, because every snippet whose
/// diagnostics fell past the cap would be attributed nothing and reported as a pass. ~keep
const DIAGNOSTIC_LIMIT: &str = "100000";

/// Package prefix that isolates one batched snippet's top-level types from every other snippet's.
/// Java resolves duplicate class names per *package*, not per source directory, so a unique
/// directory alone still makes two `Snippet` classes collide — and `wrap_if_fragment` names every
/// wrapped fragment `Snippet`. ~keep
const BATCH_PACKAGE_PREFIX: &str = "alef_snippet_";

const BATCH_DIRECTORY_PREFIX: &str = "alef-java-batch-";

/// javac diagnostic headers, as `<path>:<line>: <severity>: <message>`.
const DIAGNOSTIC_MARKERS: [(&str, DiagnosticSeverity); 2] = [
    (": error: ", DiagnosticSeverity::Error),
    (": warning: ", DiagnosticSeverity::Warning),
];

/// Lines that terminate a diagnostic block instead of continuing it: javac's trailing summary
/// (`2 errors`), its `-Werror` roll-up (`error: warnings found ...`) and `Note:` advisories all
/// start at column zero with no path, so without this they would be glued onto whichever snippet
/// happened to report last. ~keep
const DIAGNOSTIC_TERMINATORS: [&str; 3] = ["Note:", "error:", "warning:"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiagnosticSeverity {
    Error,
    Warning,
}

struct JavaDiagnostic {
    path: String,
    severity: DiagnosticSeverity,
    text: String,
}

/// One snippet's compilation unit inside a batch: a source file, plus the directory component that
/// attributes a javac diagnostic path back to it. The directory is the attribution token rather
/// than the file name because every wrapped fragment compiles to the same `Snippet.java`.
struct JavaBatchUnit {
    directory: String,
    file_name: String,
    source: String,
}

pub struct JavaValidator;

impl JavaValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        if level == ValidationLevel::Run {
            return None;
        }
        let units = Self::plan_batch(snippets, level, session)?;
        Some(Self::compile_batch(&units, level, timeout_secs, session))
    }

    /// Builds one compilation unit per snippet, or declines the batch.
    ///
    /// A snippet that declares its own `package` keeps it, because rewriting a declared package
    /// would break the qualified references and same-package imports inside it. Two such snippets
    /// sharing a package can genuinely redeclare each other's classes, and there is no reliable way
    /// to tell that apart from the compiler's own duplicate-class error — so the batch is declined
    /// and the runner falls back to one process per snippet, which cannot collide at all. ~keep
    fn plan_batch(
        snippets: &[&Snippet],
        level: ValidationLevel,
        session: Option<&ValidationSession>,
    ) -> Option<Vec<JavaBatchUnit>> {
        let fingerprint = session.map(|value| value.fingerprint.as_str());
        let mut declared_packages: HashMap<String, String> = HashMap::new();
        let mut units = Vec::with_capacity(snippets.len());
        for snippet in snippets {
            let wrapped = Self::wrap_if_fragment(&snippet.code);
            let identifier = ValidationCache::key(snippet, level, fingerprint);
            let source = match Self::declared_package(&wrapped) {
                Some(package) => {
                    let owner = declared_packages.entry(package).or_insert_with(|| identifier.clone());
                    if owner != &identifier {
                        return None;
                    }
                    wrapped.clone()
                }
                None => Self::with_package(&wrapped, &format!("{BATCH_PACKAGE_PREFIX}{identifier}")),
            };
            units.push(JavaBatchUnit {
                directory: format!("s{identifier}"),
                file_name: format!("{}.java", Self::extract_class_name(&wrapped)),
                source,
            });
        }
        Some(units)
    }

    fn compile_batch(
        units: &[JavaBatchUnit],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let root = match session {
            Some(session) => session.external_workspace_directory()?,
            None => std::env::temp_dir(),
        };
        let batch = tempfile::Builder::new()
            .prefix(BATCH_DIRECTORY_PREFIX)
            .tempdir_in(&root)
            .map_err(|error| {
                crate::snippets::error::Error::Other(format!(
                    "allocating Java snippet batch directory in {}: {error}",
                    root.display()
                ))
            })?;
        let sources = Self::write_batch_sources(units, batch.path())?;
        let classes = batch.path().join("classes");
        std::fs::create_dir_all(&classes)?;
        let mut command = Self::batch_command(&sources, &classes, level, session)?;
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(
            units,
            level == ValidationLevel::TypeCheck,
            success,
            &output,
        ))
    }

    /// Writes every unit and returns the deduplicated source list javac is invoked with. Two
    /// byte-identical snippets share an identifier and therefore a path; handing javac the same
    /// file twice is a `duplicate class` error, while writing it twice is not. ~keep
    fn write_batch_sources(units: &[JavaBatchUnit], batch: &Path) -> Result<Vec<PathBuf>> {
        let mut sources = Vec::with_capacity(units.len());
        let mut seen = HashSet::new();
        for unit in units {
            let directory = batch.join(&unit.directory);
            std::fs::create_dir_all(&directory)?;
            let path = directory.join(&unit.file_name);
            std::fs::write(&path, &unit.source)?;
            if seen.insert(path.clone()) {
                sources.push(path);
            }
        }
        Ok(sources)
    }

    fn batch_command(
        sources: &[PathBuf],
        classes: &Path,
        level: ValidationLevel,
        session: Option<&ValidationSession>,
    ) -> Result<std::process::Command> {
        let mut command = std::process::Command::new("javac");
        command.args(["-Xmaxerrs", DIAGNOSTIC_LIMIT, "-Xmaxwarns", DIAGNOSTIC_LIMIT]);
        command.arg(if level == ValidationLevel::TypeCheck {
            "-Xlint:all"
        } else {
            "-Xlint:none"
        });
        if level == ValidationLevel::TypeCheck {
            command.arg("-Werror");
        }
        command.args(["-d"]).arg(classes).args(sources);
        if let Some(value) = session {
            value.apply(&mut command);
            if let Some(manifest) = &value.manifest {
                let class_path = Self::class_path(manifest)?;
                command.args(["--class-path", class_path.to_string_lossy().as_ref()]);
            }
        }
        Ok(command)
    }

    fn batch_results(
        units: &[JavaBatchUnit],
        warnings_fail: bool,
        success: bool,
        output: &str,
    ) -> BatchValidation {
        let (diagnostics, mut unmatched) = Self::split_diagnostics(output);
        let mut attributed = vec![Vec::<String>::new(); units.len()];
        for diagnostic in diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Warning && !warnings_fail {
                continue;
            }
            let owners = Self::owning_units(units, &diagnostic.path);
            if owners.is_empty() {
                unmatched.push(diagnostic.text);
                continue;
            }
            for owner in owners {
                attributed[owner].push(diagnostic.text.clone());
            }
        }
        let attributed_any = attributed.iter().any(|messages| !messages.is_empty());
        let fallback = (!success && !attributed_any).then(|| {
            if unmatched.is_empty() {
                "javac failed without a snippet-specific diagnostic".to_string()
            } else {
                unmatched.join("\n")
            }
        });
        attributed
            .into_iter()
            .map(|messages| match (messages.is_empty(), &fallback) {
                (true, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (true, None) => (SnippetStatus::Pass, None),
                (false, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
            })
            .collect()
    }

    fn split_diagnostics(output: &str) -> (Vec<JavaDiagnostic>, Vec<String>) {
        let mut diagnostics: Vec<JavaDiagnostic> = Vec::new();
        let mut other = Vec::new();
        let mut open = false;
        for line in output.lines() {
            if let Some((path, severity)) = Self::diagnostic_header(line) {
                diagnostics.push(JavaDiagnostic {
                    path: path.to_owned(),
                    severity,
                    text: line.to_owned(),
                });
                open = true;
            } else if Self::terminates_a_diagnostic(line) {
                other.push(line.to_owned());
                open = false;
            } else if let Some(last) = diagnostics.last_mut().filter(|_| open) {
                last.text.push('\n');
                last.text.push_str(line);
            } else {
                other.push(line.to_owned());
            }
        }
        (diagnostics, other)
    }

    fn diagnostic_header(line: &str) -> Option<(&str, DiagnosticSeverity)> {
        for (marker, severity) in DIAGNOSTIC_MARKERS {
            let Some(index) = line.find(marker) else {
                continue;
            };
            let (path, number) = line[..index].rsplit_once(':')?;
            if !path.is_empty() && !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
                return Some((path, severity));
            }
        }
        None
    }

    fn terminates_a_diagnostic(line: &str) -> bool {
        DIAGNOSTIC_TERMINATORS.iter().any(|marker| line.starts_with(marker))
            || line
                .split_once(' ')
                .is_some_and(|(count, rest)| !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit()) && rest.starts_with(|c: char| c == 'e' || c == 'w'))
    }

    fn owning_units(units: &[JavaBatchUnit], path: &str) -> Vec<usize> {
        Path::new(path)
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .flat_map(|component| {
                units
                    .iter()
                    .enumerate()
                    .filter(move |(_, unit)| unit.directory == component)
                    .map(|(index, _)| index)
            })
            .collect()
    }

    fn declared_package(code: &str) -> Option<String> {
        code.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("package ")?;
            Some(rest.trim().trim_end_matches(';').trim().to_owned())
        })
    }

    fn with_package(code: &str, package: &str) -> String {
        let lines: Vec<&str> = code.lines().collect();
        let insertion = lines
            .iter()
            .position(|line| Self::is_declaration_line(line))
            .unwrap_or(lines.len());
        let mut rendered = String::new();
        for line in &lines[..insertion] {
            rendered.push_str(line);
            rendered.push('\n');
        }
        rendered.push_str(&format!("package {package};\n\n"));
        rendered.push_str(&lines[insertion..].join("\n"));
        rendered.push('\n');
        rendered
    }

    /// Whether a line is real code rather than the leading blank/comment run a `package`
    /// declaration is still allowed to follow.
    fn is_declaration_line(line: &str) -> bool {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with('*')
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let temporary_directory = session.is_none().then(ScratchDir::isolated).transpose()?;
        let directory = match (session, temporary_directory.as_ref()) {
            (Some(value), _) => value.external_workspace_directory()?,
            (None, Some(value)) => value.path().to_path_buf(),
            (None, None) => unreachable!(),
        };
        let wrapped = Self::wrap_if_fragment(&snippet.code);
        let class_name = Self::extract_class_name(&wrapped);
        let file = directory.join(format!("{class_name}.java"));
        std::fs::write(&file, &wrapped)?;
        let mut command = if level == ValidationLevel::Run {
            let mut value = std::process::Command::new("java");
            value.arg(&file);
            value
        } else {
            let mut value = std::process::Command::new("javac");
            value.arg(if level == ValidationLevel::TypeCheck {
                "-Xlint:all"
            } else {
                "-Xlint:none"
            });
            if level == ValidationLevel::TypeCheck {
                value.arg("-Werror");
            }
            value.args(["-d"]).arg(&directory).arg(&file);
            value
        };
        if let Some(value) = session {
            value.apply(&mut command);
            if let Some(manifest) = &value.manifest {
                let class_path = Self::class_path(manifest)?;
                command.args(["--class-path", class_path.to_string_lossy().as_ref()]);
            }
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn extract_class_name(code: &str) -> String {
        for line in code.lines() {
            let trimmed = line.trim();
            for keyword in ["public class ", "class ", "public final class ", "final class "] {
                if let Some(rest) = trimmed.strip_prefix(keyword) {
                    let name = rest
                        .split(|c: char| c.is_whitespace() || c == '{' || c == '<')
                        .next()
                        .unwrap_or("Snippet");
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
        "Snippet".to_string()
    }

    fn class_path(manifest: &std::path::Path) -> Result<std::ffi::OsString> {
        if manifest.is_dir() || manifest.extension().is_some_and(|extension| extension == "jar") {
            return Ok(manifest.as_os_str().to_owned());
        }
        let root = manifest.parent().unwrap_or_else(|| std::path::Path::new("."));
        let target = root.join("target");
        let mut entries = [target.join("classes"), target.join("test-classes")]
            .into_iter()
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        let dependency_directory = target.join("dependency");
        if dependency_directory.is_dir() {
            entries.extend(
                std::fs::read_dir(dependency_directory)?
                    .filter_map(std::result::Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|extension| extension == "jar")),
            );
        }
        if entries.is_empty() {
            entries.push(root.to_path_buf());
        }
        std::env::join_paths(entries).map_err(|error| {
            crate::snippets::error::Error::Other(format!("building Java classpath for {}: {error}", manifest.display()))
        })
    }

    fn has_class_or_interface(code: &str) -> bool {
        code.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("class ")
                || trimmed.starts_with("public class ")
                || trimmed.starts_with("final class ")
                || trimmed.starts_with("public final class ")
                || trimmed.starts_with("interface ")
                || trimmed.starts_with("public interface ")
                || trimmed.starts_with("public enum ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("public record ")
                || trimmed.starts_with("record ")
        })
    }

    fn split_imports(code: &str) -> (String, String) {
        let mut imports = Vec::new();
        let mut body = Vec::new();
        let mut past_imports = false;
        for line in code.lines() {
            let trimmed = line.trim();
            if !past_imports
                && (trimmed.is_empty() || trimmed.starts_with("import ") || trimmed.starts_with("package "))
            {
                imports.push(line);
            } else {
                past_imports = true;
                body.push(line);
            }
        }
        (imports.join("\n"), body.join("\n"))
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if Self::has_class_or_interface(trimmed) {
            return code.to_string();
        }

        let (imports, body) = Self::split_imports(trimmed);
        let body_trimmed = body.trim();
        let only_comments = !body_trimmed.is_empty()
            && body_trimmed
                .lines()
                .all(|line| line.trim().is_empty() || line.trim().starts_with("//"));

        let body_inner = if body_trimmed.is_empty() || only_comments {
            format!("{body_trimmed}\nint _placeholder = 0;")
        } else {
            body.to_string()
        };
        let imports_block = if imports.trim().is_empty() {
            String::new()
        } else {
            format!("{imports}\n\n")
        };
        format!(
            "{imports_block}public class Snippet {{\n    public static void main(String[] args) throws Exception {{\n{body_inner}\n    }}\n}}\n"
        )
    }
}

impl SnippetValidator for JavaValidator {
    fn language(&self) -> Language {
        Language::Java
    }

    fn is_available(&self) -> bool {
        which::which("javac").is_ok()
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

    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        Self::validate_batch_with_context(snippets, level, timeout_secs, session)
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::TypeCheck
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("cannot find symbol") || output.contains("package") && output.contains("does not exist")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn session_manifest_is_used_as_a_real_classpath() {
        let _toolchain_guard = crate::snippets::validators::jvm_toolchain_test_lock();
        if which::which("javac").is_err() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let classes = root.path().join("classes");
        let sources = root.path().join("sources/localfixture");
        std::fs::create_dir_all(&classes).expect("classes directory");
        std::fs::create_dir_all(&sources).expect("sources directory");
        let source = sources.join("Values.java");
        std::fs::write(
            &source,
            "package localfixture; public final class Values { public static final int VALUE = 1; }",
        )
        .expect("Java fixture source");
        let compiled = std::process::Command::new("javac")
            .args(["-d"])
            .arg(&classes)
            .arg(&source)
            .status()
            .expect("javac runs");
        assert!(compiled.success());
        let session = ValidationSession {
            language: Language::Java,
            working_directory: root.path().to_path_buf(),
            manifest: Some(classes),
            fingerprint: test_fingerprint(root.path()),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = JavaValidator::validate_with_context(
            &snippet("import localfixture.Values;\npublic final class Example { public static void main(String[] args) { System.out.println(Values.VALUE); } }"),
            ValidationLevel::TypeCheck,
            30,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
        cleanup_external_workspace(&session);
    }

    /// Regression test for the defect this closes: alef's own Java backend sets Maven's
    /// `<sourceDirectory>` to `${project.basedir}` (see the generated `packages/java/pom.xml`),
    /// because it emits sources at the package root rather than under `src/main/java/`. That
    /// makes *everything* under a Java session's `working_directory` a live compiler input for
    /// `mvn package`/`maven-source-plugin`/javadoc — not just a conventional `src/` subtree.
    /// Before the fix, `validate_with_context` wrote the scratch `.java` source and its compiled
    /// `.class` output to `working_directory/.alef/snippets/sessions/<fingerprint>/`, inside that
    /// globbed tree, so a snippet's throwaway source would have been compiled straight into the
    /// consumer's shipped artifact. This asserts no file appears anywhere under
    /// `working_directory` after validation. ~keep
    #[test]
    fn session_scratch_is_never_written_under_the_working_directory() {
        let _toolchain_guard = crate::snippets::validators::jvm_toolchain_test_lock();
        if which::which("javac").is_err() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let session = ValidationSession {
            language: Language::Java,
            working_directory: root.path().to_path_buf(),
            manifest: None,
            fingerprint: test_fingerprint(root.path()),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) = JavaValidator::validate_with_context(
            &snippet(
                "public final class Example { public static void main(String[] args) { System.out.println(\"ok\"); } }",
            ),
            ValidationLevel::Compile,
            30,
            Some(&session),
        )
        .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");

        let leaked_into_working_directory = walkdir::WalkDir::new(root.path())
            .into_iter()
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_type().is_file());
        assert!(
            !leaked_into_working_directory,
            "no scratch file may be written anywhere under a Java session's working_directory: alef's own Java \
             backend makes the whole directory a Maven source root"
        );
        cleanup_external_workspace(&session);
    }

    #[test]
    fn project_manifest_resolves_compiled_class_directory() {
        let project = tempfile::tempdir().expect("project directory");
        let classes = project.path().join("target/classes");
        std::fs::create_dir_all(&classes).expect("classes directory");
        let manifest = project.path().join("pom.xml");
        std::fs::write(&manifest, "<project />").expect("manifest");

        let class_path = JavaValidator::class_path(&manifest).expect("classpath");
        assert_eq!(std::env::split_paths(&class_path).collect::<Vec<_>>(), vec![classes]);
    }

    /// A fingerprint unique to this test invocation, derived from its already-unique `tempfile`
    /// root. `external_workspace_directory` is keyed by fingerprint alone (deliberately, so it
    /// can live outside any per-test tempdir) — a literal fingerprint shared across test runs
    /// would collide on the real OS temp directory instead of getting the isolation `tempfile`
    /// normally provides. ~keep
    fn test_fingerprint(root: &std::path::Path) -> String {
        root.to_string_lossy().replace(['/', '\\', ':'], "_")
    }

    fn cleanup_external_workspace(session: &ValidationSession) {
        let directory = std::env::temp_dir()
            .join("alef-snippets/sessions")
            .join(&session.fingerprint);
        let _ = std::fs::remove_dir_all(directory);
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("Example.java"),
            language: Language::Java,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("Example.java"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
