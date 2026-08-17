use super::{
    handle, refused_snippet_dir_paths, snippet_validation_needs_build_artifacts, sync_registry_versions_before_all,
    warn_if_snippet_validation_needs_build,
};
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::core::config::NewAlefConfig;

#[test]
fn all_generates_snippets_before_readmes_consume_them() {
    let source = include_str!("all_commands.rs");
    let e2e = source.find("Generating e2e test suites...").expect("e2e stage");
    let readmes = source.find("Generating READMEs...").expect("README stage");

    assert!(
        e2e < readmes,
        "README generation must observe snippets produced by the same run"
    );
}

#[test]
fn all_runs_its_only_build_step_before_the_docs_stage_that_validates_snippets() {
    let source = include_str!("all_commands.rs");
    let post_build = source
        .find("Running post-build processing...")
        .expect("post-build stage");
    let docs = source.find("Generating docs...").expect("docs stage");

    assert!(
        post_build < docs,
        "the only build `all` performs (FFI cdylib + per-backend post-build hooks, via \
         complete_generated_artifacts) runs before the docs stage that triggers snippet validation -- \
         but that build is FFI-only and does not satisfy typecheck/compile/run snippet validation for \
         languages needing a full per-language build (typescript, java, kotlin, swift, zig, ...)"
    );
}

#[test]
fn all_never_calls_the_general_per_language_build_stage() {
    let source = include_str!("all_commands.rs");

    assert!(
        !source.contains("pipeline::build("),
        "`alef all`'s documented scope (\"generate + stubs + scaffold + readme + docs + sync + e2e\") \
         excludes building native artifacts; the only build all_commands.rs may trigger is the narrow \
         FFI-only one inside `complete_generated_artifacts`. If this now calls `pipeline::build` \
         directly, `warn_if_snippet_validation_needs_build`'s precondition warning (and its doc \
         comment) is stale and must be revisited alongside this test."
    );
}

/// Regression for the incident where `alef all --clean` reported the write stage as
/// successful while the ownership guard silently refused thousands of writes.
/// `write_scaffold_files_with_overwrite` returns a bare `usize`, discarding
/// `WriteReport::refused_paths`; every e2e/test-apps/README/docs write in `alef all`
/// used it, so `refusals` -- the accumulator `report_refused_writes` reads at the end
/// of the run -- never saw a single one of those refusals, no matter how many the
/// guard logged. The standalone `alef docs` / `alef e2e generate` commands never had
/// this bug: they already write through `write_scaffold_files_report` and fold the
/// result into their own report. `alef all` must do the same for every write, not
/// just bindings/service/public-API/stubs. ~keep
#[test]
fn all_never_drops_refusals_through_the_count_only_write_wrapper() {
    let source = include_str!("all_commands.rs");
    assert!(
        !source.contains("write_scaffold_files_with_overwrite"),
        "`alef all` must write through `write_scaffold_files_report` and fold every result into \
         `refusals` via `absorb_refusals` -- the count-only wrapper silently drops refused writes, \
         which is what let a run with thousands of refusals report success"
    );
}

/// Docs/snippet validation reads its input from disk (`discover_snippets` walks
/// `docs.snippets.dirs`), not from the `doc_files` this run rendered in memory. When
/// the write just above refused to update one of those files, a validation failure
/// against the stale bytes reads as a defect in freshly generated content -- it is
/// not. The docs-stage error path must name the pending refusal count so that
/// distinction is visible at the point of failure, not only in a warning several
/// stages earlier that a reader chasing the validation error has no reason to
/// connect to it. ~keep
#[test]
fn all_correlates_a_docs_stage_failure_with_pending_write_refusals() {
    let source = include_str!("all_commands.rs");
    let doc_write = source
        .find("write_scaffold_files_report(&doc_files")
        .expect("docs write must go through the refusal-tracking writer");
    let correlation = source
        .find("Docs/snippet validation reads content from")
        .expect("doc-result error path must explain a possible refusal/stale-content correlation");
    assert!(
        doc_write < correlation,
        "the docs write must be folded into `refusals` before its failure path can correlate a \
         validation error with pending write refusals"
    );
}

#[test]
fn snippet_validation_needs_build_artifacts_is_true_only_for_toolchain_levels() {
    assert!(snippet_validation_needs_build_artifacts(Some("typecheck")));
    assert!(snippet_validation_needs_build_artifacts(Some("compile")));
    assert!(snippet_validation_needs_build_artifacts(Some("run")));
    assert!(
        snippet_validation_needs_build_artifacts(Some("Compile")),
        "the check must be case-insensitive since config values are user-authored TOML strings"
    );

    assert!(!snippet_validation_needs_build_artifacts(Some("syntax")));
    assert!(!snippet_validation_needs_build_artifacts(None));
    assert!(!snippet_validation_needs_build_artifacts(Some("bogus")));
}

fn write_config_with_snippet_validation_level(root: &std::path::Path, validation_level: &str) -> std::path::PathBuf {
    let cargo_path = root.join("Cargo.toml");
    std::fs::write(&cargo_path, "[package]\nname = \"sample-core\"\nversion = \"0.1.0\"\n").expect("write Cargo.toml");
    let config_path = root.join("alef.toml");
    let config = format!(
        concat!(
            "[workspace]\nlanguages = [\"zig\"]\n\n",
            "[workspace.docs.snippets]\nvalidation_level = {:?}\n\n",
            "[[crates]]\nname = \"sample-core\"\nsources = []\nversion_from = {:?}\n"
        ),
        validation_level,
        cargo_path.to_string_lossy(),
    );
    std::fs::write(&config_path, config).expect("write alef.toml");
    config_path
}

#[test]
fn warn_if_snippet_validation_needs_build_reads_the_merged_validation_level_without_panicking() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = write_config_with_snippet_validation_level(temp.path(), "compile");
    let configs = resolve(&config_path);
    let config = configs.into_iter().next().expect("one crate");

    let merged_level = config
        .docs
        .as_ref()
        .and_then(|docs| docs.snippets.as_ref())
        .and_then(|snippets| snippets.validation_level.as_deref());
    assert_eq!(merged_level, Some("compile"));

    // Only exercises the tracing::warn! side effect for panics; no subscriber is
    // installed in this test so nothing is asserted about the emitted message itself.
    warn_if_snippet_validation_needs_build(&config);
}

fn write_neutral_config(root: &std::path::Path, cargo_toml: &str, hash: &str) -> std::path::PathBuf {
    let cargo_path = root.join("Cargo.toml");
    std::fs::write(&cargo_path, cargo_toml).expect("write Cargo.toml");
    let config_path = root.join("alef.toml");
    let config = format!(
        concat!(
            "[workspace]\nlanguages = [\"zig\"]\n\n",
            "[[crates]]\nname = \"sample-core\"\nsources = []\nversion_from = {:?}\n\n",
            "[crates.e2e.call]\nfunction = \"sample_call\"\n\n",
            "[crates.e2e.registry.packages.zig]\n",
            "name = \"sample_pkg\"\nversion = \"0.8.0\"\nhash = {:?}\n"
        ),
        cargo_path.to_string_lossy(),
        hash
    );
    std::fs::write(&config_path, config).expect("write alef.toml");
    config_path
}

fn resolve(config_path: &std::path::Path) -> Vec<crate::core::config::ResolvedCrateConfig> {
    let raw = std::fs::read_to_string(config_path).expect("read alef.toml");
    toml::from_str::<NewAlefConfig>(&raw)
        .expect("parse alef.toml")
        .resolve()
        .expect("resolve alef.toml")
}

#[test]
fn all_preflight_repairs_stale_zig_registry_hash_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stale_hash = "sample_pkg-0.8.0-AbCd_XyZ123456789";
    let config_path = write_neutral_config(
        temp.path(),
        "[package]\nname = \"sample-core\"\nversion = \"0.9.0\"\n",
        stale_hash,
    );
    let configs = resolve(&config_path);
    let selected = configs.iter().collect::<Vec<_>>();

    let changed = sync_registry_versions_before_all(&config_path, &selected).expect("repair stale hash");

    assert!(changed);
    let repaired = std::fs::read_to_string(config_path).expect("read repaired config");
    assert!(repaired.contains("version = \"0.9.0\""));
    assert!(repaired.contains("hash = \"sample_pkg-0.9.0-AbCd_XyZ123456789\""));
}

#[test]
fn all_preflight_rejects_unreadable_version_source_without_mutating_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stale_hash = "sample_pkg-0.8.0-AbCd_XyZ123456789";
    let config_path = write_neutral_config(temp.path(), "not valid TOML", stale_hash);
    let configs = resolve(&config_path);
    let selected = configs.iter().collect::<Vec<_>>();

    let error = sync_registry_versions_before_all(&config_path, &selected).expect_err("invalid version must fail");

    assert!(
        error
            .to_string()
            .contains("could not resolve version for crate `sample-core`")
    );
    let unchanged = std::fs::read_to_string(config_path).expect("read unchanged config");
    assert!(unchanged.contains(stale_hash));
}

fn write_config_with_snippet_roots(root: &std::path::Path, dirs: &[&str], exclude: &[&str]) -> std::path::PathBuf {
    let cargo_path = root.join("Cargo.toml");
    std::fs::write(&cargo_path, "[package]\nname = \"sample-core\"\nversion = \"0.1.0\"\n").expect("write Cargo.toml");
    let config_path = root.join("alef.toml");
    let dirs_toml = dirs.iter().map(|dir| format!("{dir:?}")).collect::<Vec<_>>().join(", ");
    let exclude_toml = exclude
        .iter()
        .map(|dir| format!("{dir:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!(
        concat!(
            "[workspace]\nlanguages = [\"zig\"]\n\n",
            "[workspace.docs.snippets]\ndirs = [{}]\nexclude = [{}]\n\n",
            "[[crates]]\nname = \"sample-core\"\nsources = []\nversion_from = {:?}\n"
        ),
        dirs_toml,
        exclude_toml,
        cargo_path.to_string_lossy(),
    );
    std::fs::write(&config_path, config).expect("write alef.toml");
    config_path
}

/// Regression for the sibling half of the incident above: a refused write does not always
/// surface as a docs-stage failure. If the ownership guard refuses a write inside
/// `docs.snippets.dirs`, the stale pre-run bytes left on disk can still pass validation --
/// `discover_snippets` has no way to know they were supposed to change. Without this
/// correlation, that reads as an ordinary successful run; the invariant this closes is that
/// a validation verdict must never be reported for a file this run refused to write without
/// that fact being attributed in the output. ~keep
#[test]
fn refused_snippet_dir_paths_flags_a_refusal_inside_configured_snippet_dirs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = write_config_with_snippet_roots(temp.path(), &["docs/snippets"], &[]);
    let configs = resolve(&config_path);
    let config = configs.into_iter().next().expect("one crate");

    let refused_snippet = temp.path().join("docs/snippets/python/example.md");
    let refused_unrelated = temp.path().join("bindings/python/example.py");
    let refused_paths = std::collections::BTreeSet::from([refused_snippet.clone(), refused_unrelated]);

    let flagged = refused_snippet_dir_paths(&refused_paths, &config, temp.path());

    assert_eq!(
        flagged,
        vec![refused_snippet],
        "only the refusal inside docs.snippets.dirs must be flagged, not every refusal in the run"
    );
}

/// Normal-path counterpart: a run with no refusals inside the configured snippet roots must
/// not be flagged, so a passing docs stage still reads as an ordinary pass when validation
/// really did grade content this run rendered (or nothing changed at all). ~keep
#[test]
fn refused_snippet_dir_paths_is_empty_when_no_refusal_touches_the_snippet_roots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = write_config_with_snippet_roots(temp.path(), &["docs/snippets"], &[]);
    let configs = resolve(&config_path);
    let config = configs.into_iter().next().expect("one crate");

    let refused_unrelated = temp.path().join("bindings/python/example.py");
    let refused_paths = std::collections::BTreeSet::from([refused_unrelated]);

    assert!(refused_snippet_dir_paths(&refused_paths, &config, temp.path()).is_empty());

    assert!(
        refused_snippet_dir_paths(&std::collections::BTreeSet::new(), &config, temp.path()).is_empty(),
        "no refusals at all must never be flagged"
    );
}

/// `docs.snippets.exclude` prefixes are excluded from discovery/validation the same way
/// `docs::build_snippet_context` excludes them (see `docs/mod.rs`'s `excluded` filter) -- a
/// refusal under an excluded prefix was never going to be read by `discover_snippets`, so
/// flagging it would be a false correlation. ~keep
#[test]
fn refused_snippet_dir_paths_respects_configured_exclude_prefixes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = write_config_with_snippet_roots(temp.path(), &["docs/snippets"], &["docs/snippets/generated"]);
    let configs = resolve(&config_path);
    let config = configs.into_iter().next().expect("one crate");

    let refused_excluded = temp.path().join("docs/snippets/generated/example.md");
    let refused_paths = std::collections::BTreeSet::from([refused_excluded]);

    assert!(refused_snippet_dir_paths(&refused_paths, &config, temp.path()).is_empty());
}

/// Structural counterpart to `all_correlates_a_docs_stage_failure_with_pending_write_refusals`:
/// that test proves the failure arm attributes a refused write to the docs-stage error. This
/// proves the success arm does the same -- `refused_snippet_dir_paths` must be consulted inside
/// `match doc_result`'s `Ok` arm, before the `Err` arm begins, so a passing docs stage still gets
/// checked (and the check does not accidentally end up inside the `Err` arm instead). ~keep
#[test]
fn all_checks_for_refused_snippet_writes_on_the_docs_stage_success_path() {
    let source = include_str!("all_commands.rs");
    let ok_arm = source.find("Ok(()) => {").expect("docs stage Ok arm");
    let success_check = source
        .find("refused_snippet_dir_paths(&refusals.refused_paths")
        .expect("docs stage success path must consult refused_snippet_dir_paths");
    let err_arm = source.find("Err(error) => {").expect("docs stage Err arm");

    assert!(
        ok_arm < success_check,
        "the success-path refusal check must live inside `match doc_result`'s `Ok` arm"
    );
    assert!(
        success_check < err_arm,
        "the success-path refusal check must run before the `Err` arm begins, not inside it"
    );
}

/// Regression for the defect where a docs/snippet validation failure returned out of `handle`
/// immediately -- `return` exits the enclosing *function*, not just the current loop iteration
/// (`for` loops in Rust have no `return`-equivalent of their own), so that early return also
/// skipped every later crate in a multi-crate run, not merely the rest of the failing crate's own
/// stages. By the time the docs stage runs, the bindings a crate wrote are already on disk (the
/// write stages all precede it); returning early there left them permanently unformatted and
/// unstamped, and an unstamped file has no provenance marker for the ownership guard to
/// recognise next run -- silently manufacturing that run's refusal set. The fix defers the
/// failure into `docs_stage_error` instead of returning it in place, so this asserts the `Err`
/// arm contains no `return` at all between its start and the point it records the deferred
/// error. ~keep
#[test]
fn all_docs_stage_failure_does_not_return_before_formatting_and_hash_stamping() {
    let source = include_str!("all_commands.rs");
    let err_arm_start = source.find("Err(error) => {").expect("docs stage Err arm");
    let err_arm_end = source
        .find("docs_stage_error.get_or_insert(error);")
        .expect("the docs stage Err arm must defer via docs_stage_error");
    let err_arm_body = &source[err_arm_start..err_arm_end];

    assert!(
        !err_arm_body.contains("return"),
        "the docs-stage `Err` arm must defer the failure via `docs_stage_error`, not `return` -- a \
         `return` here exits `handle` immediately, skipping formatting, orphan sweeping, hash \
         finalisation, deferred-formatting reporting and hook installation for this crate and every \
         later crate in this loop. Arm body was: {err_arm_body:?}"
    );

    let format_generated = source
        .find("pipeline::format_generated(&files_to_format, resolved_cfg, &base_dir, None)")
        .expect("the converging whole-tree formatting pass must still run after the docs stage");
    let finalize_hashes_sweeping = source
        .find("pipeline::finalize_hashes_sweeping(")
        .expect("hash stamping must still run after the docs stage");
    let sweep_manifest_orphans = source
        .find("pipeline::sweep_manifest_orphans(&previous_paths, &current_gen_paths, &cleanup_roots, &cleanup_roots)")
        .expect("orphan sweeping must still run after the docs stage");

    assert!(
        err_arm_end < sweep_manifest_orphans,
        "orphan sweeping must be reachable after the docs-stage `Err` arm completes"
    );
    assert!(
        err_arm_end < format_generated,
        "formatting must be reachable after the docs-stage `Err` arm completes"
    );
    assert!(
        err_arm_end < finalize_hashes_sweeping,
        "hash stamping must be reachable after the docs-stage `Err` arm completes"
    );
    // Load-bearing ordering (see the `~keep` comments at each call site in all_commands.rs, each
    // written after a real bug): the orphan sweep must run before hash finalisation because
    // `finalize_hashes_sweeping` clones rather than mutates `current_gen_paths`, and `None` must
    // stay the third argument to `format_generated` to keep the converging whole-tree pass instead
    // of the single-pass branch. This restructure must not have disturbed either. ~keep
    assert!(
        sweep_manifest_orphans < finalize_hashes_sweeping,
        "sweep_manifest_orphans must still run before finalize_hashes_sweeping"
    );
    // The exact-literal search for `format_generated` above already pins its third argument to
    // `None` (the converging whole-tree pass) -- a change to `Some(&changed_languages)` would have
    // made that `.expect(...)` panic rather than let this test silently check the wrong call.
}

/// The deferred docs-stage error must still fail the overall run, and only after every
/// must-always-run step -- formatting, orphan sweeping, hash finalisation, deferred-formatting
/// reporting, hook installation, and the run-level refusal report -- has executed for every
/// crate, not before. Propagating it any earlier than the end of `handle` would reintroduce the
/// short-circuit this restructure removes. ~keep
#[test]
fn all_propagates_the_deferred_docs_error_only_after_hook_installation() {
    let source = include_str!("all_commands.rs");
    let install_hooks = source
        .find("pipeline::install_poly_hooks(&base_dir);")
        .expect("hook installation stage");
    let propagate = source
        .find("if let Some(error) = docs_stage_error {")
        .expect("the deferred docs error must be propagated once, after the loop");

    assert!(
        install_hooks < propagate,
        "the deferred docs-stage error must be returned only after hook installation (and every \
         other must-always-run step) has completed for every crate, not before"
    );

    let tail = &source[propagate..];
    assert!(
        tail.contains("return Err(error);"),
        "the deferred error must be returned as-is -- it already carries whatever `.context(...)` \
         the `Err` arm applied (the refusal-count wrapping), so this must not rebuild or discard it"
    );
    assert!(
        !source[..propagate].contains("return Err(error.context"),
        "the docs-stage `Err` arm must not return directly -- `.context(...)` is applied while \
         building the deferred `error` binding, not at a `return` site"
    );
}

// ---------------------------------------------------------------------------
// `e2e_stage_error` deferral -- drives the real `handle` entry point
// ---------------------------------------------------------------------------
//
// `crate::e2e::generate_e2e`'s own unit tests (in `src/e2e/mod.rs`) call it directly and
// can never observe this defect: the discard-everything-on-one-backend's-failure bug lived
// in the CALLER's `?`, not inside `generate_e2e` itself. Only driving the real `handle`
// entry point can prove the caller no longer does that. ~keep

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
        strict: false,
        skip_frb: false,
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
