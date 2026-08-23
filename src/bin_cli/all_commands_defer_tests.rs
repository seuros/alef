//! Regression coverage for task #186's "all-or-nothing abort" defect: a single rejected
//! fixture, or a single crate's post-build failure, must never deny every other `alef all`
//! stage -- or every other crate -- its regeneration. Both tests drive the real `handle` entry
//! point (not a hand-built stub of the orchestration) because the defect lived in the CALLER's
//! `?`/`return Err`, not in any function these tests could exercise directly.

use super::handle;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::test_support::CwdGuard;

fn all_command() -> Commands {
    Commands::All {
        clean: false,
        clobber_create_once_seeds: false,
        strict: false,
        skip_frb: false,
        skip_snippet_validation: false,
    }
}

fn expect_err(result: anyhow::Result<Option<Commands>>, message: &str) -> anyhow::Error {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{message}"),
    }
}

// ---------------------------------------------------------------------------
// Pre-flight snippet-coverage precondition must not abort the main loop
// ---------------------------------------------------------------------------

const PREFLIGHT_FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";

const PREFLIGHT_FIXTURE_CARGO_TOML: &str =
    "[package]\nname = \"preflightlib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

const PREFLIGHT_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "preflightlib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["rust"]

[crates.e2e.call]
function = "greet"
module = "preflightlib"
result_var = "result"

[[crates.e2e.call.args]]
name = "name"
field = "input.name"
type = "string"

[crates.e2e.snippets]
output = "docs/snippets"
"#;

const PREFLIGHT_FIXTURE_DOCUMENTED: &str = r#"{
  "id": "greet_basic",
  "description": "Greets someone",
  "category": "smoke",
  "tags": ["smoke"],
  "input": {"name": "Ada"},
  "assertions": [{"type": "not_error"}],
  "docs": {"topic": "guides"}
}
"#;

/// No `"docs"` key at all -- `crate::e2e::snippets::generate_snippet_report` records this as
/// `coverage.missing` ("fixture has no documentation metadata") rather than rendering a
/// snippet. One such gap is enough to fail `ensure_snippet_coverage_complete`, which is the
/// same code path a rejected mock-harness-guard fixture reaches (both flow through
/// `evaluate_snippet_coverage`/`ensure_fresh_snippet_coverage_complete` in `handle`'s pre-flight
/// loop) -- this fixture reproduces the identical orchestration defect with a trigger that
/// needs no toolchain, no extension, and no fixture-generator internals to set up. ~keep
const PREFLIGHT_FIXTURE_UNDOCUMENTED: &str = r#"{
  "id": "greet_no_docs",
  "description": "Greets someone else",
  "category": "smoke",
  "tags": ["smoke"],
  "input": {"name": "Grace"},
  "assertions": [{"type": "not_error"}]
}
"#;

fn write_preflight_fixture_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::create_dir_all(root.join("fixtures")).expect("create fixture fixtures directory");
    std::fs::write(root.join("src/lib.rs"), PREFLIGHT_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), PREFLIGHT_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("fixtures/greet_basic.json"), PREFLIGHT_FIXTURE_DOCUMENTED)
        .expect("write documented fixture");
    std::fs::write(root.join("fixtures/greet_no_docs.json"), PREFLIGHT_FIXTURE_UNDOCUMENTED)
        .expect("write undocumented fixture");
    std::fs::write(root.join("alef.toml"), PREFLIGHT_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// THE DEFECT: before this fix, a coverage gap discovered by `handle`'s pre-flight loop
/// (`evaluate_snippet_coverage`/`ensure_fresh_snippet_coverage_complete`, run once per crate
/// before the main generation loop) propagated with `?` immediately -- aborting the run before
/// bindings, e2e suites, READMEs or docs for ANY crate were ever generated. A consumer repo hit
/// this for real: a single rejected fixture out of 264 meant `alef all` wrote nothing at all,
/// and every stage had to be invoked by hand to get a regen (task #186). ~keep
#[test]
fn a_preflight_coverage_gap_does_not_abort_the_main_generation_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_preflight_fixture_workspace(&root);
    let _cwd = CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let error = expect_err(
        handle(all_command(), &context),
        "a real coverage gap must still fail the run -- writing everything else must not turn \
         this into a healthy exit code",
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("snippet coverage precondition"),
        "the failure must name the pre-flight stage that found the gap: {message}"
    );
    assert!(
        message.contains("greet_no_docs"),
        "the failure must carry the underlying coverage-gap diagnostic verbatim: {message}"
    );

    let bindings = root.join("packages/python/preflightlib/__init__.py");
    assert!(
        bindings.is_file(),
        "python bindings must still have been generated despite the pre-flight coverage gap: {} is missing",
        bindings.display()
    );
    let rust_cargo_toml = root.join("e2e").join("rust").join("Cargo.toml");
    assert!(
        rust_cargo_toml.is_file(),
        "the e2e stage must still have run and written its rust suite despite the pre-flight \
         coverage gap: {} is missing",
        rust_cargo_toml.display()
    );
    let readme = root.join("packages/python/README.md");
    assert!(
        readme.is_file(),
        "the README stage must still have run despite the pre-flight coverage gap: {} is missing",
        readme.display()
    );
}

// ---------------------------------------------------------------------------
// A crate's post-build failure must not abort its own remaining stages
// ---------------------------------------------------------------------------

const POST_BUILD_FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";

const POST_BUILD_FIXTURE_CARGO_TOML: &str =
    "[package]\nname = \"postbuildlib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

/// `languages = ["ffi"]` with no `[workspace] ... members` covering the generated
/// `crates/postbuildlib-ffi` directory: `complete_generated_artifacts` always runs
/// `cargo build -p postbuildlib-ffi` for an FFI-configured crate (see
/// `bin_cli::helpers::complete_generated_artifacts`), and with no workspace declaring that
/// package, `cargo` fails fast (no compilation, no network) with "package ID specification
/// ... did not match any packages" -- a real, deterministic post-build failure with no
/// toolchain-availability flakiness. ~keep
const POST_BUILD_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "postbuildlib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["rust"]

[crates.e2e.call]
function = "greet"
module = "postbuildlib"
result_var = "result"

[[crates.e2e.call.args]]
name = "name"
field = "input.name"
type = "string"
"#;

const POST_BUILD_FIXTURE_JSON: &str = r#"{
  "id": "greet_basic",
  "description": "Greets someone",
  "category": "smoke",
  "tags": ["smoke"],
  "input": {"name": "Ada"},
  "assertions": [{"type": "not_error"}]
}
"#;

fn write_post_build_fixture_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::create_dir_all(root.join("fixtures")).expect("create fixture fixtures directory");
    std::fs::write(root.join("src/lib.rs"), POST_BUILD_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), POST_BUILD_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("fixtures/greet_basic.json"), POST_BUILD_FIXTURE_JSON).expect("write fixture json");
    std::fs::write(root.join("alef.toml"), POST_BUILD_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// THE DEFECT: before this fix, `complete_generated_artifacts`'s `Err` propagated via
/// `return Err(error)` straight out of `handle` -- so a single crate's post-build failure
/// (real-world: a Dart `flutter_rust_bridge_codegen` break) meant stubs, e2e, READMEs and docs
/// for THAT crate never ran, and neither did any crate listed after it. A consumer repo hit
/// this for real (task #186): the run hard-stopped after the generate stage, with
/// e2e/test-apps files left at pre-session mtimes. This fixture reproduces the same shape
/// against alef's own tests, holding `SKIP_COMMANDS_LOCK` for its whole duration because the
/// `cargo build` below must genuinely run and fail, not be silently skipped by a concurrent
/// test's `ALEF_SKIP_COMMANDS=cargo` (see `SkipCommandsGuard`'s doc).
///
/// This run has exactly one recorded stage failure, so `StageFailures::into_result` returns the
/// original `anyhow::Error` unchanged rather than wrapping it in a `"[crate] post-build
/// processing: ..."` summary line (see that type's own unit tests) -- the `"[postbuildlib] post-
/// build processing failed"` label is only guaranteed to appear in the real-time log, which
/// `#[traced_test]` captures below. ~keep
#[tracing_test::traced_test]
#[test]
fn a_crate_post_build_failure_does_not_abort_its_own_remaining_stages() {
    let _skip_guard = crate::test_support::SkipCommandsGuard::set("");
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_post_build_fixture_workspace(&root);
    let _cwd = CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let error = expect_err(
        handle(all_command(), &context),
        "a genuine post-build failure must still fail the run -- writing everything else must \
         not turn this into a healthy exit code",
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("postbuildlib-ffi"),
        "the failure must carry the underlying cargo diagnostic verbatim: {message}"
    );
    assert!(
        logs_contain("[postbuildlib] post-build processing"),
        "the post-build stage must still be named in the real-time log, even though the deferred \
         error's own text is returned unchanged"
    );

    let rust_cargo_toml = root.join("e2e").join("rust").join("Cargo.toml");
    assert!(
        rust_cargo_toml.is_file(),
        "the e2e stage must still have run and written its rust suite despite the post-build \
         failure: {} is missing",
        rust_cargo_toml.display()
    );
    let readme = root.join("crates/postbuildlib-ffi/README.md");
    assert!(
        readme.is_file(),
        "the README stage must still have run despite the post-build failure: {} is missing",
        readme.display()
    );
}
