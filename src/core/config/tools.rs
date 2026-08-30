//! Global tooling configuration.
//!
//! `[tools]` is a top-level section in `alef.toml` that selects per-language
//! package managers and dev-tool sets used by the default pipeline commands
//! (lint, test, build, setup, update, clean). Each field has a sensible default
//! so the section is fully optional; users only override what they need.
//!
//! One exception to "each field has a default": an explicitly set `python_package_manager` also
//! redirects the Python *build* through that manager's locked environment, while an unset one
//! leaves the build resolving maturin off `PATH`. [`super::python_build::python_tool_runner`]
//! therefore reads the raw field rather than [`ToolsConfig::python_pm`] — inventing a package
//! manager a repo never asked for would hand it an unrunnable build command. ~keep

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default Rust dev tools installed by `alef setup rust`.
/// Mirrors the polyrepo's `task setup` so binding generators get a consistent
/// developer environment out of the box.
pub const DEFAULT_RUST_DEV_TOOLS: &[&str] = &[
    "cargo-edit",
    "cargo-sort",
    "cargo-machete",
    "cargo-deny",
    "cargo-llvm-cov",
];

const DEFAULT_PYTHON_PM: &str = "uv";
const DEFAULT_NODE_PM: &str = "pnpm";

/// Top-level `[tools]` config. Selects which package manager / tool variants
/// the default per-language pipeline commands target.
///
/// All fields are optional; getters return the documented default when unset.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolsConfig {
    /// Python package manager. One of: `"uv"`, `"pip"`, `"poetry"`. Default: `"uv"`.
    #[serde(default)]
    pub python_package_manager: Option<String>,

    /// Node package manager. One of: `"pnpm"`, `"npm"`, `"yarn"`. Default: `"pnpm"`.
    #[serde(default)]
    pub node_package_manager: Option<String>,

    /// Rust dev tools installed by the Rust `setup` default.
    /// Default: see [`DEFAULT_RUST_DEV_TOOLS`].
    #[serde(default)]
    pub rust_dev_tools: Option<Vec<String>>,
}

/// Per-language context passed to every `default_*_config` function.
///
/// Bundles the global `[tools]` selection plus three optional knobs that
/// reduce override boilerplate in consumer `alef.toml` files:
///
/// - `run_wrapper` — prefix every default tool invocation, e.g. wrap
///   `ruff format …` with `uv run --no-sync` so the lint step inherits the
///   project's package-manager environment without a full override.
/// - `extra_lint_paths` — append additional paths to the default lint
///   commands (`format`, `check`, `typecheck`).
/// - `project_file` — for languages whose tools target a project descriptor
///   (Java's `pom.xml`, C#'s `.csproj`/`.slnx`), use this file instead of
///   the package directory.
#[derive(Debug, Clone)]
pub struct LangContext<'a> {
    pub tools: &'a ToolsConfig,
    pub run_wrapper: Option<&'a str>,
    pub extra_lint_paths: &'a [String],
    pub project_file: Option<&'a str>,
}

impl<'a> LangContext<'a> {
    /// Create a context with all knobs unset (no wrapper, no extra paths,
    /// no project file). Useful in tests and call sites that only need the
    /// global tools selection.
    pub fn default(tools: &'a ToolsConfig) -> Self {
        Self {
            tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        }
    }
}

/// Wrap `cmd` with `wrapper` (e.g. `uv run --no-sync`) when set.
///
/// Used by per-language defaults so a single project-level knob can prefix
/// every default tool invocation without forcing a full command override.
pub fn wrap_command(cmd: String, wrapper: Option<&str>) -> String {
    match wrapper {
        Some(w) => format!("{w} {cmd}"),
        None => cmd,
    }
}

/// Append space-separated `paths` to `cmd`. No-op when `paths` is empty.
///
/// Path entries are inserted verbatim into a string that reaches `sh -c` (via the
/// `lint`/`build`/`test`/`update`/`setup` default builders that call this). This function
/// itself does no escaping -- the guarantee comes from upstream, not from this call site.
/// `super::validation::validate_extra_lint_paths` rejects any `extra_lint_paths` entry outside
/// `[A-Za-z0-9._/-]+` once, at config resolution, before any `Language` config carrying one
/// reaches a default builder. A previous version of this comment claimed that check existed
/// when it did not; re-derive this claim from `super::validation`'s actual contents rather
/// than trusting the comment, the same way that gap was found.
pub fn append_paths(cmd: String, paths: &[String]) -> String {
    if paths.is_empty() {
        cmd
    } else {
        format!("{} {}", cmd, paths.join(" "))
    }
}

/// Build a POSIX precondition that checks whether `tool` is on `PATH`.
///
/// The resulting command exits 0 when the tool is available and non-zero
/// otherwise. Used by per-language defaults so a missing tool causes a
/// graceful warn-and-skip rather than a hard failure.
pub fn require_tool(tool: &str) -> String {
    format!("command -v {tool} >/dev/null 2>&1")
}

/// Build a POSIX precondition requiring multiple tools to be on `PATH`.
///
/// Joins individual `command -v` checks with `&&` so the precondition only
/// passes when every listed tool is present.
pub fn require_tools(tools: &[&str]) -> String {
    tools.iter().map(|t| require_tool(t)).collect::<Vec<_>>().join(" && ")
}

/// Require the selected Ruby interpreter to resolve its own Bundler executable. ~keep
pub fn require_ruby_bundler() -> String {
    format!(
        "{} && {} >/dev/null 2>&1",
        require_tool("ruby"),
        ruby_bundle("--version")
    )
}

/// Run Bundler with a project-local gem path separated by Ruby ABI. ~keep
pub(crate) fn ruby_bundle(arguments: &str) -> String {
    format!("BUNDLE_PATH=vendor/bundle ruby -S bundle {arguments}")
}

/// Run a bundled Ruby gem executable through the active Ruby interpreter. ~keep
pub(crate) fn ruby_bundle_exec(command: &str) -> String {
    ruby_bundle(&format!("exec ruby -S {command}"))
}

impl ToolsConfig {
    /// Resolved Python package manager (defaults to `uv` when unset).
    pub fn python_pm(&self) -> &str {
        self.python_package_manager.as_deref().unwrap_or(DEFAULT_PYTHON_PM)
    }

    /// Resolved Node package manager (defaults to `pnpm` when unset).
    pub fn node_pm(&self) -> &str {
        self.node_package_manager.as_deref().unwrap_or(DEFAULT_NODE_PM)
    }

    /// Resolved Rust dev tools (defaults to [`DEFAULT_RUST_DEV_TOOLS`] when unset).
    pub fn rust_tools(&self) -> Vec<&str> {
        match self.rust_dev_tools.as_deref() {
            Some(list) => list.iter().map(String::as_str).collect(),
            None => DEFAULT_RUST_DEV_TOOLS.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        let cfg = ToolsConfig::default();
        assert_eq!(cfg.python_pm(), "uv");
        assert_eq!(cfg.node_pm(), "pnpm");
        assert_eq!(
            cfg.rust_tools(),
            vec![
                "cargo-edit",
                "cargo-sort",
                "cargo-machete",
                "cargo-deny",
                "cargo-llvm-cov"
            ]
        );
    }

    #[test]
    fn getters_return_user_value_when_set() {
        let cfg = ToolsConfig {
            python_package_manager: Some("pip".to_string()),
            node_package_manager: Some("yarn".to_string()),
            rust_dev_tools: Some(vec!["cargo-foo".to_string(), "cargo-bar".to_string()]),
        };
        assert_eq!(cfg.python_pm(), "pip");
        assert_eq!(cfg.node_pm(), "yarn");
        assert_eq!(cfg.rust_tools(), vec!["cargo-foo", "cargo-bar"]);
    }

    #[test]
    fn empty_rust_dev_tools_is_respected() {
        let cfg = ToolsConfig {
            rust_dev_tools: Some(vec![]),
            ..Default::default()
        };
        assert!(cfg.rust_tools().is_empty());
    }

    #[test]
    fn deserializes_from_toml() {
        let toml_str = r#"
            python_package_manager = "poetry"
            node_package_manager = "npm"
            rust_dev_tools = ["cargo-edit"]
        "#;
        let cfg: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.python_pm(), "poetry");
        assert_eq!(cfg.node_pm(), "npm");
        assert_eq!(cfg.rust_tools(), vec!["cargo-edit"]);
    }

    #[test]
    fn require_tool_emits_command_v() {
        assert_eq!(require_tool("ruff"), "command -v ruff >/dev/null 2>&1");
    }

    #[test]
    fn ruby_bundler_precondition_checks_the_active_interpreter() {
        assert_eq!(
            require_ruby_bundler(),
            "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
        );
    }

    #[test]
    fn ruby_bundle_exec_forces_bundler_and_gem_tool_through_active_interpreter() {
        assert_eq!(
            ruby_bundle_exec("rubocop -A ."),
            "BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rubocop -A ."
        );
    }

    #[cfg(unix)]
    fn write_executable(path: &std::path::Path, content: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::write(path, content).expect("write executable");
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod executable");
    }

    #[cfg(unix)]
    #[test]
    fn ruby_bundle_exec_survives_foreign_tool_shebangs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ruby = temp.path().join("ruby");
        let bundle = temp.path().join("bundle");
        let rubocop = temp.path().join("rubocop");
        let marker = temp.path().join("marker");
        write_executable(
            &ruby,
            "#!/bin/sh\n[ \"$1\" = -S ] || exit 91\nshift\nscript=$(command -v \"$1\") || exit 92\nshift\nexec /bin/sh \"$script\" \"$@\"\n",
        );
        write_executable(
            &bundle,
            "#!/missing/foreign/ruby\n[ \"$BUNDLE_PATH\" = vendor/bundle ] || exit 94\n[ \"$1\" = exec ] || exit 93\nshift\nexec \"$@\"\n",
        );
        write_executable(
            &rubocop,
            "#!/missing/foreign/ruby\nprintf '%s\\n' active > \"$ABI_PROBE\"\n",
        );
        let path = format!(
            "{}:{}",
            temp.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let run = |command: &str| {
            std::process::Command::new("/bin/sh")
                .args(["-c", command])
                .env("PATH", &path)
                .env("ABI_PROBE", &marker)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("run ABI probe")
        };

        assert!(!run("BUNDLE_PATH=vendor/bundle bundle exec ruby -S rubocop").success());
        assert!(!run("ruby -S bundle exec ruby -S rubocop").success());
        assert!(!run("BUNDLE_PATH=vendor/bundle ruby -S bundle exec rubocop").success());
        assert!(!marker.exists());
        assert!(run(&ruby_bundle_exec("rubocop")).success());
        assert_eq!(std::fs::read_to_string(marker).expect("read marker"), "active\n");
    }

    #[test]
    fn require_tools_joins_with_and() {
        assert_eq!(
            require_tools(&["go", "gofmt"]),
            "command -v go >/dev/null 2>&1 && command -v gofmt >/dev/null 2>&1"
        );
    }

    #[test]
    fn empty_toml_uses_defaults() {
        let cfg: ToolsConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.python_pm(), "uv");
        assert_eq!(cfg.node_pm(), "pnpm");
    }
}
