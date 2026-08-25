//! `src/snippets/validators/python.rs` runs pyrefly, but only over isolated doc-snippet
//! scratch files -- never over a real generated package. A type checker alef ships and never
//! points at its own generated output is a check that examines nothing: the pyo3 backend's
//! public-dataclass-vs-native-pyclass mismatch (task alef-310, fixed alongside this test) went
//! undetected by every automated gate alef runs on itself, and was only found by a human running
//! pyrefly by hand over a real consumer's `packages/python`. This test closes that gap by driving
//! `alef all` end to end against a real fixture -- the same dispatch path a consumer runs -- and
//! then running pyrefly over the actual `packages/python` directory it wrote, exactly as a
//! consumer's own `pyrefly check` (or `alef lint`'s `typecheck` step, see
//! `core::config::lint_defaults::default_lint_config`) would.
//!
//! This is a single dedicated test, not a step wired into every `cargo test` or `alef build`:
//! pyrefly is an external tool this dev machine happens to have, most CI machines running the
//! full `--lib` suite do not, and a type-checker pass over generated Python is exactly the kind
//! of slow, environment-dependent check that belongs in one targeted place rather than on every
//! build. It follows the same `which::which("pyrefly").is_err() { return }` skip convention
//! already used by `snippets::validators::python`'s own pyrefly-backed tests, rather than
//! inventing a second one.

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;

const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

/// `maybe_result` returns `Option<ResultData>` -- never `ResultData` bare -- so it exercises the
/// extraction fix (`is_return_type` must be set through `Option`/`Vec`/`Map` wrappers, not only
/// on a bare `Named` return) exactly as much as it exercises the pyo3 codegen that reads that
/// flag. `ResultData` has no other use in this fixture, so if the flag were wrong `options.py`
/// would emit it as an input dataclass while `api.py` actually returns/receives the native
/// pyclass -- the mismatch pyrefly's `bad-return` catches.
///
/// The remaining constructs below (`Point`, `list_points`, `Shape`, `ValidationError`) exist to
/// audit the three still-suppressed pyrefly codes (`bad-argument-count`, `not-iterable`,
/// `missing-attribute`; task alef-334) against real generated output rather than leaving them
/// suppressed on faith:
///
/// - `Point::new`/`Point::translate` take several parameters including an `Option<...>` one, the
///   shape most likely to desync the wrapper `def`'s parameter count from the native pyclass
///   constructor/method it calls (`bad-argument-count`).
/// - `list_points` returns `Vec<Point>`, the shape most likely to produce generated code that
///   iterates, indexes, or unpacks a collection pyrefly cannot prove is iterable
///   (`not-iterable`).
/// - `Shape` is a data-carrying enum and `ValidationError` is a field-carrying error type, both
///   shapes most likely to produce generated attribute access across the wrapper/native boundary
///   that pyrefly cannot prove exists (`missing-attribute`). ~keep
const FIXTURE_SOURCE: &str = r#"
#[derive(Default)]
pub struct ResultData {
    pub label: String,
}

pub fn maybe_result(flag: bool) -> Option<ResultData> {
    if flag {
        Some(ResultData { label: "found".to_string() })
    } else {
        None
    }
}

#[derive(Default, Clone)]
pub struct Point {
    pub x: i64,
    pub y: i64,
    pub label: Option<String>,
}

impl Point {
    pub fn new(x: i64, y: i64, label: Option<String>) -> Self {
        Point { x, y, label }
    }

    pub fn translate(&self, dx: i64, dy: i64, scale: Option<i64>) -> Point {
        let factor = scale.unwrap_or(1);
        Point {
            x: self.x + dx * factor,
            y: self.y + dy * factor,
            label: self.label.clone(),
        }
    }
}

pub fn list_points(count: i64) -> Vec<Point> {
    (0..count).map(|i| Point::new(i, i, None)).collect()
}

pub enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
}

pub fn describe_shape(shape: Shape) -> String {
    match shape {
        Shape::Circle { radius } => format!("circle r={radius}"),
        Shape::Rectangle { width, height } => format!("rect {width}x{height}"),
    }
}

#[derive(Debug)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

pub fn validate(value: i64) -> Result<i64, ValidationError> {
    if value < 0 {
        Err(ValidationError {
            field: "value".to_string(),
            message: "must be non-negative".to_string(),
        })
    } else {
        Ok(value)
    }
}
"#;

const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
"#;

fn write_fixture_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// The scaffolded `pyproject.toml` lives beside the python package directory (e.g.
/// `packages/python/pyproject.toml` next to `packages/python/test_lib/`), not inside it -- find
/// it by its `[tool.pyrefly]` marker rather than hard-coding the package-name-derived path.
fn find_pyrefly_project_dir(root: &std::path::Path) -> std::path::PathBuf {
    for entry in walkdir_pyproject_tomls(root) {
        let content = std::fs::read_to_string(&entry).unwrap_or_default();
        if content.contains("[tool.pyrefly]") {
            return entry
                .parent()
                .expect("pyproject.toml has a parent directory")
                .to_path_buf();
        }
    }
    panic!("no scaffolded pyproject.toml with a [tool.pyrefly] section found under {root:?}");
}

fn walkdir_pyproject_tomls(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == "pyproject.toml") {
                found.push(path);
            }
        }
    }
    found
}

/// Runs `alef all` against a real fixture, then runs real pyrefly 1.2.0+ over the real
/// `packages/python` output it wrote -- the same directory and the same `pyproject.toml` (with
/// its scaffolded `[[tool.pyrefly.sub-config]]` suppressions) a consumer's own `pyrefly check`
/// would see. Zero errors is the regression gate: a return-type or argument-type boundary
/// mismatch reintroduced into the pyo3 backend surfaces here as a real `bad-return` or
/// `bad-argument-type`, not just in a hand-run consumer audit.
#[test]
fn alef_all_generated_python_package_type_checks_clean_under_pyrefly() {
    if which::which("pyrefly").is_err() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    super::handle(
        Commands::All {
            clean: false,
            clobber_create_once_seeds: false,
            strict: false,
            skip_frb: false,
            skip_snippet_validation: true,
        },
        &context,
    )
    .expect("alef all must succeed against a plain python fixture");

    let api_py = root.join("packages/python/test_lib/api.py");
    assert!(
        api_py.is_file(),
        "sanity: alef all must have written api.py, got tree under {root:?}"
    );

    let project_dir = find_pyrefly_project_dir(&root);

    let output = std::process::Command::new("pyrefly")
        .arg("check")
        .arg(&project_dir)
        .output()
        .expect("pyrefly check must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "pyrefly must report zero errors against alef's own generated package \
         (a `bad-return`/`bad-argument-type` here means the public-dataclass boundary fix \
         regressed); pyrefly stdout:\n{stdout}\npyrefly stderr:\n{stderr}"
    );
}
