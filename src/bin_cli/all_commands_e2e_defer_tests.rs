// ---------------------------------------------------------------------------
// `e2e_stage_error` deferral -- drives the real `handle` entry point
// ---------------------------------------------------------------------------
//
// `crate::e2e::generate_e2e`'s own unit tests (in `src/e2e/mod.rs`) call it directly and
// can never observe this defect: the discard-everything-on-one-backend's-failure bug lived
// in the CALLER's `?`, not inside `generate_e2e` itself. Only driving the real `handle`
// entry point can prove the caller no longer does that. ~keep

use super::handle;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;

/// `handle`'s "All" arm resolves every relative path it touches (fixtures, e2e output,
/// `.alef/` cache, `version_from`) against `std::env::current_dir()`, not against the
/// config file's directory (see `let base_dir = std::env::current_dir()?;` at the top of
/// the "All" arm) -- so driving it against an isolated fixture requires actually changing
/// the process's working directory. `crate::test_support::CwdGuard` serializes that against
/// every other cwd-mutating test in the crate, not only the ones in this binary. ~keep
use crate::test_support::CwdGuard as E2eDeferCwdGuard;

const E2E_DEFER_FIXTURE_SOURCE: &str = r#"
pub struct Metadata {
    pub document_title: String,
}

pub struct CompletionResult {
    pub id: String,
    pub metadata: Metadata,
}

pub fn complete(prompt: String) -> Result<CompletionResult, String> {
    let _ = prompt;
    Err("unimplemented".to_string())
}
"#;

const E2E_DEFER_FIXTURE_CARGO_TOML: &str = "[package]\nname = \"deferlib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

/// `languages = ["c", "rust"]` deliberately omits every other backend: `c` is the only
/// generator that runs `ensure_leaf_field_exists` (`e2e/codegen/c/assertions.rs`), and
/// `rust` is the sibling this test proves keeps generating despite the `c` failure. No
/// `[crates.ffi]` / `ffi` in `[workspace] languages` on purpose -- scaffolding `ffi` would
/// make `alef all` build a real FFI cdylib via `complete_generated_artifacts`, which this
/// fixture crate (no `Cargo.toml` dependencies, not a real workspace member) cannot do;
/// `codegen::generators_for` selects the `c` e2e generator by name alone; it does not
/// require `ffi` to be scaffolded. ~keep
const E2E_DEFER_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "deferlib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["c", "rust"]

[crates.e2e.call]
function = "complete"
module = "deferlib"
result_var = "result"

[[crates.e2e.call.args]]
name = "prompt"
field = "input.prompt"
type = "string"
"#;

/// `field` names the path the fixture asserts on. `"metadata.document_title"` is a real
/// field of `Metadata` and generates cleanly on every backend. `"metadata.title"` is the
/// one-hop-then-missing-leaf shape `ensure_leaf_field_exists` rejects -- `Metadata` is a
/// real IR type (`parent_is_ir_type`), the leaf is not declared in
/// `[crates.e2e.fields_c_types]`, and it names no field of `Metadata`. Only the `c`
/// backend performs this check, so `rust` succeeds either way -- which is what makes it a
/// usable sibling-backend probe. ~keep
fn e2e_defer_fixture_json(field: &str) -> String {
    format!(
        "{{\n  \"id\": \"complete_basic\",\n  \"description\": \"a completion asserting a field on \
         the nested Metadata type\",\n  \"category\": \"smoke\",\n  \"tags\": [\"smoke\"],\n  \
         \"call\": \"_default\",\n  \"input\": {{ \"prompt\": \"hello\" }},\n  \"assertions\": [\n    \
         {{ \"type\": \"not_error\" }},\n    {{ \"type\": \"equals\", \"field\": \"{field}\", \"value\": \"irrelevant\" }}\n  \
         ]\n}}\n"
    )
}

fn write_e2e_defer_fixture_workspace(root: &std::path::Path, field: &str) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::create_dir_all(root.join("fixtures")).expect("create fixture fixtures directory");
    std::fs::write(root.join("src/lib.rs"), E2E_DEFER_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), E2E_DEFER_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("fixtures/complete_basic.json"), e2e_defer_fixture_json(field))
        .expect("write fixture json");
    std::fs::write(root.join("alef.toml"), E2E_DEFER_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

fn e2e_defer_all_command() -> Commands {
    Commands::All {
        clean: false,
        clobber_create_once_seeds: false,
        strict: false,
        skip_frb: false,
        skip_snippet_validation: false,
        skip_compile: false,
    }
}

/// `Commands` derives only `clap::Subcommand`, not `Debug`, so `Result::expect_err` (which
/// needs `Debug` on the `Ok` side to build its panic message) cannot be used directly on a
/// `Result<Option<Commands>, _>`. ~keep
fn expect_e2e_defer_err(result: anyhow::Result<Option<Commands>>, message: &str) -> anyhow::Error {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{message}"),
    }
}

/// Regression for the defect this whole change fixes: a C-backend codegen failure
/// (`ensure_leaf_field_exists`) must not discard the sibling `rust` backend's already-
/// generated e2e suite, and it must still fail the run -- a passing exit code with the
/// failing backend's suite silently absent would be worse than today's all-or-nothing
/// failure, not a fix. Both halves are asserted together because a naive fix could satisfy
/// either one alone (write everything and always return `Ok`, or keep failing hard and
/// write nothing). ~keep
#[test]
fn all_defers_an_e2e_generator_failure_so_sibling_backends_still_write_and_the_run_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_e2e_defer_fixture_workspace(&root, "metadata.title");
    let _cwd = E2eDeferCwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let error = expect_e2e_defer_err(
        handle(e2e_defer_all_command(), &context),
        "a C-backend e2e codegen failure must still fail the run -- writing sibling files must \
         not silently turn this into a healthy exit code",
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("e2e codegen failed") && message.contains("[c]"),
        "the propagated error must be the deferred e2e generator failure naming the `c` backend, \
         not something else: {message}"
    );
    assert!(
        message.contains("Metadata") && message.contains("title"),
        "the failure must carry `ensure_leaf_field_exists`'s own diagnostic verbatim: {message}"
    );

    let rust_cargo_toml = root.join("e2e").join("rust").join("Cargo.toml");
    assert!(
        rust_cargo_toml.is_file(),
        "the `rust` sibling backend must still have written its e2e suite even though the `c` \
         backend's codegen failed: {} is missing",
        rust_cargo_toml.display()
    );
    let c_makefile = root.join("e2e").join("c").join("Makefile");
    assert!(
        !c_makefile.is_file(),
        "the failing `c` backend itself must not have produced output -- `run_generators` \
         treats a backend's `Err` as zero files for that backend, not a partial write: {}",
        c_makefile.display()
    );
}

/// The hazard a naive version of this fix would introduce, and the reason it is worse than
/// the bug it replaces: `sweep_manifest_orphans` and `cache::write_stage_hash` must not run
/// on a deferred generator failure. Un-gated, `write_stage_hash` would record the failing
/// attempt as cached, so the *next* run reads `[e2e] up to date (skipping)`, never calls
/// `generate_e2e` again, and silently exits 0 with the `c` suite permanently missing; and
/// `sweep_manifest_orphans` would compare this run's incomplete path set against the last
/// good run's complete one and delete the previously-working `c` output as "orphaned". ~keep
#[test]
fn all_gates_e2e_stage_hash_and_orphan_sweep_on_a_deferred_generator_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_e2e_defer_fixture_workspace(&root, "metadata.document_title");
    let _cwd = E2eDeferCwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    // Run 1: every field the fixture asserts on is real, so the `c` backend succeeds and
    // its output, plus the `e2e` stage hash, are recorded as this run's known-good state.
    handle(e2e_defer_all_command(), &context).expect("the baseline run with a valid field path must succeed");

    let c_makefile = root.join("e2e").join("c").join("Makefile");
    assert!(
        c_makefile.is_file(),
        "the baseline run must have produced `c` e2e output at {}",
        c_makefile.display()
    );

    // Rewrite the fixture to assert a field `Metadata` does not have. This changes the
    // fixtures directory's hash, so the `e2e` stage cannot read as cached on the next run
    // and `generate_e2e` runs again, this time hitting the `c` backend's failure.
    std::fs::write(
        root.join("fixtures/complete_basic.json"),
        e2e_defer_fixture_json("metadata.title"),
    )
    .expect("rewrite fixture with a field Metadata does not have");

    expect_e2e_defer_err(
        handle(e2e_defer_all_command(), &context),
        "a run whose fixture trips ensure_leaf_field_exists must still fail",
    );

    assert!(
        c_makefile.is_file(),
        "the previously-good `c` output must survive a run whose `c` backend failed -- \
         sweep_manifest_orphans must not run on a deferred generator failure, or it deletes the \
         last known-good backend output: {}",
        c_makefile.display()
    );

    // Run again over the SAME broken fixture: nothing changed since the previous attempt,
    // so the `e2e` stage hash this run computes is identical to it. If `write_stage_hash`
    // had been called on that failed attempt, this run would now read `[e2e] up to date
    // (skipping)`, skip `generate_e2e` entirely, and return `Ok` -- silently masking the
    // failure from here on.
    expect_e2e_defer_err(
        handle(e2e_defer_all_command(), &context),
        "a repeat run over the same broken fixture must still fail -- a stage hash written on \
         the previous failed attempt would silently cache the failure away",
    );
}

// ---------------------------------------------------------------------------
// Task #362: a HARD abort in e2e generation must not leave already-written,
// already-formatted binding output unstamped.
// ---------------------------------------------------------------------------
//
// Every test above this point pins the SOFT `generator_error` case, which `all_commands.rs`
// already caught and deferred before this fix (a single backend's codegen failed but
// `generate_e2e` itself still returned `Ok`). This section drives the other half: a fatal
// error out of `crate::e2e::generate_e2e(..)?`'s own outer `Result` -- e.g. a malformed
// fixtures directory -- which used to `?`/`bail!` straight out of `handle`, skipping this
// crate's terminal `format_generated_reporting` + `finalize_hashes_sweeping` pass entirely
// and leaving every binding/stub/public-API/scaffold file this run had already written with
// no `alef:hash:` line at all. A unit test on `finalize_hashes` in isolation cannot see this:
// the bug is that the call is never reached, not that it stamps incorrectly. ~keep

/// Two fixture files sharing the same `id` -- `load_fixtures` (`src/e2e/fixture.rs`) rejects
/// this with a hard `bail!` *before* any backend codegen or write starts. That failure
/// surfaces through the outer `Result` on `crate::e2e::generate_e2e(..)?`, not through the
/// softer `generator_error` the tests above already cover. ~keep
fn write_e2e_defer_duplicate_id_fixture_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::create_dir_all(root.join("fixtures")).expect("create fixture fixtures directory");
    std::fs::write(root.join("src/lib.rs"), E2E_DEFER_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), E2E_DEFER_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    let fixture_json = e2e_defer_fixture_json("metadata.document_title");
    std::fs::write(root.join("fixtures/a.json"), &fixture_json).expect("write first fixture json");
    std::fs::write(root.join("fixtures/b.json"), &fixture_json).expect("write duplicate-id fixture json");
    std::fs::write(root.join("alef.toml"), E2E_DEFER_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// Regression for task #362: a fatal (non-`generator_error`) e2e codegen failure must still
/// defer through `e2e_stage_error` -- exactly like the softer failure above -- so this crate's
/// python binding, written earlier in the same `alef all` run, still reaches the terminal
/// format+stamp pass and carries a well-formed `alef:hash:` line. Before the fix, the
/// duplicate-fixture-ID `bail!` propagated out of `handle` via `?` before
/// `finalize_hashes_sweeping` ever ran for this crate, so the binding was written and
/// formatted but never stamped -- present on disk, invisible to `alef verify`. ~keep
#[test]
fn all_stamps_bindings_written_before_a_hard_e2e_abort() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_e2e_defer_duplicate_id_fixture_workspace(&root);
    let _cwd = E2eDeferCwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let error = expect_e2e_defer_err(
        handle(e2e_defer_all_command(), &context),
        "a duplicate fixture ID must still fail the run",
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("duplicate fixture ID"),
        "the propagated error must be the `load_fixtures` duplicate-ID failure, not something \
         else: {message}"
    );

    let binding_source = root.join("crates/deferlib-py/src/lib.rs");
    assert!(
        binding_source.is_file(),
        "the python binding this run wrote before the e2e stage's hard abort must exist on disk: {}",
        binding_source.display()
    );
    let content = std::fs::read_to_string(&binding_source).expect("read the written binding");
    assert!(
        crate::core::hash::extract_hash(&content).is_some(),
        "a fatal error in e2e generation must not leave already-written, already-formatted \
         binding output with no `alef:hash:` line -- `alef verify` cannot then distinguish this \
         alef-generated file from a hand-authored one. Binding content was:\n{content}"
    );
}
