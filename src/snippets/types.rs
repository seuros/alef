use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Bash,
    C,
    Csharp,
    Dart,
    Docker,
    Elixir,
    Go,
    Java,
    Json,
    Kotlin,
    Mermaid,
    Php,
    PowerShell,
    Python,
    R,
    Ruby,
    Rust,
    Swift,
    Text,
    Toml,
    TypeScript,
    Xml,
    Yaml,
    Zig,
    Unknown,
}

impl Language {
    #[must_use]
    pub fn from_fence_tag(tag: &str) -> Self {
        match tag.trim().to_lowercase().as_str() {
            "bash" | "sh" | "shell" | "zsh" | "console" => Self::Bash,
            "c" => Self::C,
            "csharp" | "c#" | "cs" => Self::Csharp,
            "dart" => Self::Dart,
            "docker" | "dockerfile" => Self::Docker,
            "elixir" | "ex" | "exs" => Self::Elixir,
            "go" | "golang" => Self::Go,
            "java" => Self::Java,
            "json" => Self::Json,
            "kotlin" | "kt" | "kts" => Self::Kotlin,
            "mermaid" => Self::Mermaid,
            "php" => Self::Php,
            "powershell" | "ps" | "ps1" | "pwsh" => Self::PowerShell,
            "python" | "py" | "python3" => Self::Python,
            "r" | "rscript" => Self::R,
            "ruby" | "rb" => Self::Ruby,
            "rust" | "rs" => Self::Rust,
            "swift" => Self::Swift,
            "text" | "txt" | "plain" => Self::Text,
            "toml" => Self::Toml,
            "typescript" | "ts" | "javascript" | "js" => Self::TypeScript,
            "xml" => Self::Xml,
            "yaml" | "yml" => Self::Yaml,
            "zig" => Self::Zig,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub fn from_session_target(target: &str) -> Self {
        match Self::normalize_session_target(target).as_str() {
            "node" | "wasm" => Self::TypeScript,
            "kotlin_android" => Self::Kotlin,
            "core" | "rust_core" => Self::Rust,
            "c_ffi" | "ffi" => Self::C,
            other => Self::from_fence_tag(other),
        }
    }

    #[must_use]
    pub fn normalize_session_target(target: &str) -> String {
        target.trim().to_lowercase().replace('-', "_")
    }

    #[must_use]
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "sh" | "bash" => Self::Bash,
            "c" | "h" => Self::C,
            "cs" => Self::Csharp,
            "dart" => Self::Dart,
            "dockerfile" => Self::Docker,
            "ex" | "exs" => Self::Elixir,
            "go" => Self::Go,
            "java" => Self::Java,
            "json" => Self::Json,
            "kt" | "kts" => Self::Kotlin,
            "php" => Self::Php,
            "py" => Self::Python,
            "r" => Self::R,
            "rb" => Self::Ruby,
            "rs" => Self::Rust,
            "swift" => Self::Swift,
            "toml" => Self::Toml,
            "ts" | "js" | "mts" | "mjs" => Self::TypeScript,
            "zig" => Self::Zig,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub fn from_dir_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "bash" | "shell" => Self::Bash,
            "c" => Self::C,
            "csharp" | "c-sharp" | "dotnet" => Self::Csharp,
            "dart" => Self::Dart,
            "docker" => Self::Docker,
            "elixir" => Self::Elixir,
            "go" | "golang" => Self::Go,
            "java" => Self::Java,
            "json" => Self::Json,
            "kotlin" | "kotlin_android" | "kotlin-android" => Self::Kotlin,
            "php" => Self::Php,
            "python" => Self::Python,
            "r" => Self::R,
            "ruby" => Self::Ruby,
            "rust" => Self::Rust,
            "swift" => Self::Swift,
            "toml" => Self::Toml,
            "typescript" | "wasm" | "node" => Self::TypeScript,
            "zig" => Self::Zig,
            _ => Self::Unknown,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bash => write!(f, "bash"),
            Self::C => write!(f, "c"),
            Self::Csharp => write!(f, "csharp"),
            Self::Dart => write!(f, "dart"),
            Self::Docker => write!(f, "docker"),
            Self::Elixir => write!(f, "elixir"),
            Self::Go => write!(f, "go"),
            Self::Java => write!(f, "java"),
            Self::Json => write!(f, "json"),
            Self::Kotlin => write!(f, "kotlin"),
            Self::Mermaid => write!(f, "mermaid"),
            Self::Php => write!(f, "php"),
            Self::PowerShell => write!(f, "powershell"),
            Self::Python => write!(f, "python"),
            Self::R => write!(f, "r"),
            Self::Ruby => write!(f, "ruby"),
            Self::Rust => write!(f, "rust"),
            Self::Swift => write!(f, "swift"),
            Self::Text => write!(f, "text"),
            Self::Toml => write!(f, "toml"),
            Self::TypeScript => write!(f, "typescript"),
            Self::Xml => write!(f, "xml"),
            Self::Yaml => write!(f, "yaml"),
            Self::Zig => write!(f, "zig"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

impl std::str::FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let language = Self::from_fence_tag(s);
        if language == Self::Unknown {
            Err(format!("unknown language: {s}"))
        } else {
            Ok(language)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationLevel {
    Syntax,
    Compile,
    /// Static type-checking without executing the code (e.g. `mypy` for Python, `tsc` for
    /// TypeScript). Deeper than `Compile` for dynamically-typed languages whose compile step is
    /// only a bytecode/syntax pass; equivalent to `Compile` for languages whose compiler already
    /// type-checks. Ordered between `Compile` and `Run` so it is the strongest static guarantee
    /// short of execution. ~keep
    TypeCheck,
    Run,
}

impl fmt::Display for ValidationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => write!(f, "syntax"),
            Self::Compile => write!(f, "compile"),
            Self::TypeCheck => write!(f, "typecheck"),
            Self::Run => write!(f, "run"),
        }
    }
}

impl std::str::FromStr for ValidationLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "syntax" => Ok(Self::Syntax),
            "compile" => Ok(Self::Compile),
            "typecheck" | "type-check" => Ok(Self::TypeCheck),
            "run" => Ok(Self::Run),
            _ => Err(format!("unknown validation level: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnippetAnnotationKind {
    Skip,
    CompileOnly,
    SyntaxOnly,
    TypeCheckOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetAnnotation {
    pub kind: SnippetAnnotationKind,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnippetMetadata {
    pub id: Option<String>,
    pub language: Option<Language>,
    pub target: Option<String>,
    pub title: Option<String>,
    pub level: Option<ValidationLevel>,
    pub skip: bool,
    pub reason: Option<String>,
    pub tags: Vec<String>,
    pub requires: Vec<String>,
    pub side_effect: Option<SideEffectClass>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideEffectClass {
    #[serde(alias = "none", alias = "local")]
    Safe,
    Network,
    Process,
    Install,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnippetStatus {
    Pass,
    Downgraded,
    Fail,
    Skip,
    Error,
    Unavailable,
}

impl fmt::Display for SnippetStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(f, "pass"),
            Self::Downgraded => write!(f, "downgraded"),
            Self::Fail => write!(f, "fail"),
            Self::Skip => write!(f, "skip"),
            Self::Error => write!(f, "error"),
            Self::Unavailable => write!(f, "unavailable"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: Option<String>,
    pub path: PathBuf,
    pub language: Language,
    pub title: Option<String>,
    pub code: String,
    pub start_line: usize,
    pub block_index: usize,
    pub annotation: Option<SnippetAnnotation>,
    pub metadata: SnippetMetadata,
    pub source_origin: SourceOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceOrigin {
    pub path: PathBuf,
    pub line: usize,
    pub block_index: usize,
}

/// Why a result's effective level fell below the requested level, or why a `Pass` needed a
/// caveat at all. Distinct from `SnippetStatus`: a `capability_capped` `Pass` and a `Downgraded`
/// result can share a reason (`ValidatorCapability`), and two `Downgraded` results can differ
/// (`Annotation` vs `Environment`) — attribution needs the reason, not just the status, to tell a
/// consumer what to actually do about it. ~keep
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DowngradeReason {
    /// A front-matter `level:` contract was requested and fully satisfied — reported for
    /// attribution even though the status is `Pass`, not a violation. ~keep
    Declared,
    /// A `<!-- snippet:*-only -->` suppression annotation lowered the ceiling below what was
    /// requested; the author's choice, so it still fails strict. ~keep
    Annotation,
    /// The validator can never reach the requested level for this language (`max_level`, or a
    /// structural `achievable_level` gap) — unsatisfiable in any environment.
    ValidatorCapability,
    /// This run's environment could not back the requested level, but a different environment
    /// could (e.g. a real type-checker binary happens to be missing).
    Environment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub snippet: Snippet,
    pub status: SnippetStatus,
    pub level: ValidationLevel,
    pub requested_level: ValidationLevel,
    pub effective_level: ValidationLevel,
    pub message: Option<String>,
    pub duration_ms: u64,
    /// True when the snippet passed below the requested level solely because its validator
    /// declares a lower `max_level`. That ceiling is a capability statement, not a quality
    /// signal, so strict mode must not treat it as a failure — otherwise requesting a level
    /// any validator caps below is structurally unsatisfiable. Downgrades from any other
    /// cause leave this false and still fail strict. ~keep
    #[serde(default)]
    pub capability_capped: bool,
    /// Populated whenever the effective level differs from the requested level for a reason
    /// worth naming — `None` for an ordinary, unqualified `Pass`, and equally for `Fail`, `Skip`,
    /// `Error`, or `Unavailable`, none of which have a reason in this taxonomy at all. `None` is
    /// deliberately a real "not applicable" here rather than a degraded default: the only writer
    /// that ever sets this to `Some` is `runner::finalize_result` (via `classify_result`), which
    /// is exhaustive over every path that produces `Downgraded` or a `capability_capped` `Pass`
    /// — see the `debug_assert!` there. ~keep
    #[serde(default)]
    pub downgrade_reason: Option<DowngradeReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub schema_version: u32,
    pub total: usize,
    pub passed: usize,
    pub downgraded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub unavailable: usize,
    /// Passing snippets whose level was limited by their validator's declared ceiling.
    /// Reported so a strict run can say what it accepted rather than hiding it. ~keep
    #[serde(default)]
    pub capability_capped: usize,
    pub results: Vec<ValidationResult>,
}

impl RunSummary {
    #[must_use]
    pub fn from_results(results: Vec<ValidationResult>) -> Self {
        let mut summary = Self {
            schema_version: 1,
            total: results.len(),
            passed: 0,
            downgraded: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
            unavailable: 0,
            capability_capped: 0,
            results,
        };

        for result in &summary.results {
            if result.capability_capped {
                summary.capability_capped += 1;
            }
            match result.status {
                SnippetStatus::Pass => summary.passed += 1,
                SnippetStatus::Downgraded => summary.downgraded += 1,
                SnippetStatus::Fail => summary.failed += 1,
                SnippetStatus::Skip => summary.skipped += 1,
                SnippetStatus::Error => summary.errors += 1,
                SnippetStatus::Unavailable => summary.unavailable += 1,
            }
        }

        summary
    }

    #[must_use]
    pub const fn has_failures(&self) -> bool {
        self.failed > 0 || self.errors > 0
    }
}

#[cfg(test)]
mod tests {
    use super::{SideEffectClass, SnippetAnnotationKind, ValidationLevel};

    #[test]
    fn validation_level_parses_typecheck_aliases() {
        assert_eq!("typecheck".parse::<ValidationLevel>(), Ok(ValidationLevel::TypeCheck));
        assert_eq!("type-check".parse::<ValidationLevel>(), Ok(ValidationLevel::TypeCheck));
        assert_eq!("TypeCheck".parse::<ValidationLevel>(), Ok(ValidationLevel::TypeCheck));
        assert_eq!(ValidationLevel::TypeCheck.to_string(), "typecheck");
    }

    #[test]
    fn typecheck_orders_between_compile_and_run() {
        assert!(ValidationLevel::Compile < ValidationLevel::TypeCheck);
        assert!(ValidationLevel::TypeCheck < ValidationLevel::Run);
    }

    #[test]
    fn typecheck_only_annotation_kind_is_distinct() {
        assert_ne!(SnippetAnnotationKind::TypeCheckOnly, SnippetAnnotationKind::CompileOnly);
    }

    #[test]
    fn side_effects_round_trip_and_accept_legacy_safe_aliases() {
        for class in [
            SideEffectClass::Safe,
            SideEffectClass::Network,
            SideEffectClass::Process,
            SideEffectClass::Install,
            SideEffectClass::Server,
        ] {
            let encoded = serde_json::to_string(&class).unwrap();
            assert_eq!(serde_json::from_str::<SideEffectClass>(&encoded).unwrap(), class);
        }
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""none""#).unwrap(),
            SideEffectClass::Safe
        );
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""local""#).unwrap(),
            SideEffectClass::Safe
        );
        assert!(serde_json::from_str::<SideEffectClass>(r#""external_mutation""#).is_err());
    }
}
