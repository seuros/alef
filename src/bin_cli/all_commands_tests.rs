use super::{
    refused_snippet_dir_paths, snippet_validation_needs_build_artifacts, sync_registry_versions_before_all,
    warn_if_snippet_validation_needs_build,
};
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
        .find("pipeline::sweep_manifest_orphans(&previous_paths, &current_gen_paths, &cleanup_roots)")
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
