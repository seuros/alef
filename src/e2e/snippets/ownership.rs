//! Durable, per-path proof that alef authored a generated snippet.
//!
//! ## The deadlock this exists to break
//!
//! A fixture snippet is a `.md` with `generated_header: false`. `write::marker_comment_style`
//! returns `None` for `.md`, so `write_scaffold_files_report`'s ownership guard cannot use the
//! markable rail, and the snippet output root was never registered in `.alef-ownership.toml`
//! (that record is populated only when the guard itself *creates* a path). Since
//! `render_snippet_markdown` began routing through `docs::with_html_header`, every snippet alef
//! writes carries a marker — but a snippet population that predates that change carries none,
//! and the guard refuses to write the very bytes that would supply one. The refusal is
//! permanent and self-perpetuating: 2,820 of 2,894 refusals in one consumer repo, 1,350 of
//! 1,351 in another, unchanged run after run.
//!
//! ## Why the ledger is the right evidence
//!
//! `.alef-snippet-coverage.json` already records, per path, "I personally wrote this exact file
//! for this fixture/language cell" — `coverage::orphaned_paths`'s doc says so in those words,
//! and alef already **deletes** files on the strength of that record. Refusing to *overwrite*
//! the same recorded paths while being willing to unlink them is incoherent; this module closes
//! that gap by reading the same field (`generated_metadata`) that the delete path trusts.
//!
//! This is a record, not an inference. The standing objection at
//! `cli::pipeline::generate::scaffold`'s guard — "byte-equality with generated output is not
//! evidence of authorship" — is aimed at content-equivalence predicates that cannot tell an
//! older-release alef file from a hand-written coincidence. It does not reach a ledger entry:
//! the path is in the ledger *because* a previous run rendered a snippet to it
//! (`generate_snippet_report_with_extensions` pushes the entry on the same iteration that builds
//! the `GeneratedFile`), which is authorship by construction. Plain byte-equality would in any
//! case be vacuous on exactly this population: generated output now carries a marker the frozen
//! files lack, so it can never compare equal to them.
//!
//! ## Why the snapshot is taken before the run writes anything
//!
//! `e2e::run` pushes this run's freshly computed ledger into the same `all_files` batch as the
//! snippets, and `write_scaffold_files_report` writes in `BTreeMap` path order, where
//! `.alef-snippet-coverage.json` sorts ahead of every sibling snippet directory. Reading the
//! ledger lazily from the guard would therefore read *this run's intentions*, not the previous
//! run's record — which degrades the rule to bare path identity ("alef is generating this path"),
//! the exact predicate that nearly destroyed a consumer's hand-written 408-line Java class at a
//! path alef also generates. [`snapshot_pre_run_ledger`] is called at the top of snippet
//! generation, before any write, so the set is pinned to what alef had already written when the
//! run started. First snapshot of a process wins, so a repeated generate in one process cannot
//! widen it.
//!
//! ## Fail-closed
//!
//! [`is_ledger_owned_snippet_path`] consults the snapshot only — never disk. No snapshot, an
//! unreadable or wrong-version ledger, a ledger entry that escapes its root, or a non-`.md`
//! entry all yield "not owned", which leaves the guard's existing refusal in place. ~keep

use super::{COVERAGE_MANIFEST, COVERAGE_MANIFEST_VERSION, SnippetCoverageLedger};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type LedgerSnapshots = Mutex<BTreeMap<PathBuf, BTreeSet<PathBuf>>>;

static PRE_RUN_LEDGERS: OnceLock<LedgerSnapshots> = OnceLock::new();

fn snapshots() -> &'static LedgerSnapshots {
    PRE_RUN_LEDGERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Pin the snippet paths a *previous* run recorded under `output_root`.
///
/// `output_root` is the repo-relative `docs.snippets.output` value, resolved against the process
/// working directory exactly as `e2e::run`'s orphan pruning resolves it. Idempotent per root, and
/// deliberately first-write-wins: a second generate pass in the same process must not be able to
/// promote its own intentions into ownership. ~keep
pub fn snapshot_pre_run_ledger(output_root: &Path) {
    let mut snapshots = match snapshots().lock() {
        Ok(snapshots) => snapshots,
        Err(poisoned) => poisoned.into_inner(),
    };
    if snapshots.contains_key(output_root) {
        return;
    }
    let owned = read_owned_paths(output_root);
    tracing::debug!(
        target: "alef::e2e::snippets::ownership",
        root = %output_root.display(),
        recorded = owned.len(),
        "snapshotted pre-run snippet ownership ledger"
    );
    snapshots.insert(output_root.to_path_buf(), owned);
}

/// True when `path` is a Markdown snippet a previous alef run recorded as its own output.
///
/// `base_dir` is the crate root the write guard joins its relative paths against, so
/// `base_dir.join(output_root)` reconstructs the same absolute prefix lexically — no
/// canonicalisation, because `write_scaffold_files_report` builds its `full_path` by the same
/// lexical join and a canonicalised comparison would silently stop matching on any tree reached
/// through a symlink. ~keep
pub fn is_ledger_owned_snippet_path(base_dir: &Path, path: &Path) -> bool {
    if !is_markdown(path) {
        return false;
    }
    let snapshots = match snapshots().lock() {
        Ok(snapshots) => snapshots,
        Err(poisoned) => poisoned.into_inner(),
    };
    snapshots.iter().any(|(output_root, owned)| {
        path.strip_prefix(base_dir.join(output_root))
            .is_ok_and(|relative| owned.contains(relative))
    })
}

/// Read `generated_metadata` — the field `coverage::orphaned_paths` already treats as the sole
/// record of "alef wrote this exact path" — rather than the parallel `generated_paths` array.
/// `coverage::validate_generated_metadata` proves the two agree on any ledger alef accepts, so
/// this is the same set; taking it from the field the delete path trusts keeps a single answer to
/// "which paths does alef own" instead of two that could drift. ~keep
fn read_owned_paths(output_root: &Path) -> BTreeSet<PathBuf> {
    let manifest = output_root.join(COVERAGE_MANIFEST);
    let Ok(bytes) = std::fs::read(&manifest) else {
        return BTreeSet::new();
    };
    let ledger: SnippetCoverageLedger = match serde_json::from_slice(&bytes) {
        Ok(ledger) => ledger,
        Err(error) => {
            tracing::warn!(
                target: "alef::e2e::snippets::ownership",
                manifest = %manifest.display(),
                "snippet coverage ledger is unreadable, so no pre-existing snippet can prove alef \
                 ownership this run: {error}"
            );
            return BTreeSet::new();
        }
    };
    if ledger.format_version != COVERAGE_MANIFEST_VERSION {
        tracing::warn!(
            target: "alef::e2e::snippets::ownership",
            manifest = %manifest.display(),
            found = ledger.format_version,
            expected = COVERAGE_MANIFEST_VERSION,
            "snippet coverage ledger version is unsupported, so it establishes no ownership"
        );
        return BTreeSet::new();
    }
    ledger
        .generated_metadata
        .into_iter()
        .map(|entry| entry.path)
        .filter(|path| is_claimable_entry(path))
        .collect()
}

/// A ledger entry may only name a plain relative Markdown file beneath its own root.
///
/// The extension gate is not cosmetic: `snippet_path` emits `.md` and nothing else, so any other
/// extension in the ledger is corruption or tampering, and refusing it categorically means a
/// ledger can never be the instrument that claims a `.java`/`.go`/`.json` file. The component
/// gate rejects `..`, absolute paths and Windows prefixes so a ledger cannot reach outside the
/// snippet root at all. ~keep
fn is_claimable_entry(path: &Path) -> bool {
    is_markdown(path)
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("md")
}

#[cfg(test)]
mod tests {
    use super::super::{GeneratedSnippetMetadata, SnippetCoverageKey};
    use super::*;
    use crate::e2e::fixture::SideEffectClass;

    fn metadata(path: &str) -> GeneratedSnippetMetadata {
        GeneratedSnippetMetadata {
            key: SnippetCoverageKey {
                fixture_id: "example".into(),
                language: "python".into(),
            },
            path: PathBuf::from(path),
            language: "python".into(),
            target: "python".into(),
            session: "python".into(),
            requires: Vec::new(),
            side_effect: SideEffectClass::Safe,
        }
    }

    fn ledger(version: u32, paths: &[&str]) -> SnippetCoverageLedger {
        SnippetCoverageLedger {
            format_version: version,
            generated_paths: paths.iter().map(PathBuf::from).collect(),
            generated_metadata: paths.iter().map(|path| metadata(path)).collect(),
            ..SnippetCoverageLedger::default()
        }
    }

    /// Each test gets its own base directory, so its snapshot key is unique and the process-global
    /// map cannot carry state between tests regardless of ordering or parallelism. ~keep
    fn snapshot(ledger: &SnippetCoverageLedger) -> (tempfile::TempDir, PathBuf) {
        let base = tempfile::tempdir().expect("temporary base directory");
        let output_root = PathBuf::from("docs/snippets");
        let absolute_root = base.path().join(&output_root);
        std::fs::create_dir_all(&absolute_root).expect("create snippet root");
        std::fs::write(
            absolute_root.join(COVERAGE_MANIFEST),
            serde_json::to_vec(ledger).expect("serialize ledger"),
        )
        .expect("write ledger");
        snapshot_pre_run_ledger(&absolute_root);
        (base, absolute_root)
    }

    #[test]
    fn a_path_the_previous_run_recorded_is_claimable() {
        let (base, root) = snapshot(&ledger(COVERAGE_MANIFEST_VERSION, &["python/chat/smoke.md"]));

        assert!(is_ledger_owned_snippet_path(
            base.path(),
            &root.join("python/chat/smoke.md")
        ));
    }

    /// **Load-bearing half of the fix.** Everything above buys the right to overwrite thousands of
    /// files; this is the only thing standing between that right and a consumer's hand-maintained
    /// content. One real tree carries 293 hand-written `id: legacy_*` snippets under the *same*
    /// top-level snippet root as 2,820 generated ones, so "inside the snippet root" can never be
    /// the test — only "named by the ledger" can. A regression here is silent and destroys
    /// hand-authored documentation, so this test must fail loudly rather than be relaxed. ~keep
    #[test]
    fn a_hand_written_snippet_under_the_same_root_is_not_claimable() {
        let (base, root) = snapshot(&ledger(COVERAGE_MANIFEST_VERSION, &["python/chat/smoke.md"]));

        assert!(
            !is_ledger_owned_snippet_path(base.path(), &root.join("legacy/migration-guide.md")),
            "an unrecorded snippet under the snippet root must never be claimed"
        );
        assert!(
            !is_ledger_owned_snippet_path(base.path(), &root.join("python/chat/hand-written.md")),
            "an unrecorded snippet in a recorded snippet's own directory must never be claimed"
        );
    }

    /// The near-miss this fix is designed around: a hand-written 408-line Java public API class at
    /// a path alef also generates. A snippet ledger must be structurally incapable of naming it,
    /// whatever the ledger says. ~keep
    #[test]
    fn a_non_markdown_ledger_entry_is_never_claimable() {
        let (base, root) = snapshot(&ledger(
            COVERAGE_MANIFEST_VERSION,
            &["packages/java/PublicApi.java", "packages/node/package.json"],
        ));

        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &root.join("packages/java/PublicApi.java")
        ));
        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &root.join("packages/node/package.json")
        ));
    }

    #[test]
    fn a_ledger_entry_escaping_its_root_is_never_claimable() {
        let (base, root) = snapshot(&ledger(COVERAGE_MANIFEST_VERSION, &["../../README.md"]));

        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &base.path().join("README.md")
        ));
        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &root.join("../../README.md")
        ));
    }

    /// A path outside every snapshotted root shares no ownership with the snippet tree, even when
    /// it is Markdown and even when its tail matches a recorded entry. ~keep
    #[test]
    fn a_markdown_file_outside_the_snapshotted_root_is_not_claimable() {
        let (base, _root) = snapshot(&ledger(COVERAGE_MANIFEST_VERSION, &["python/chat/smoke.md"]));

        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &base.path().join("docs/other/python/chat/smoke.md")
        ));
    }

    #[test]
    fn an_unsupported_ledger_version_establishes_no_ownership() {
        let (base, root) = snapshot(&ledger(COVERAGE_MANIFEST_VERSION + 1, &["python/chat/smoke.md"]));

        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &root.join("python/chat/smoke.md")
        ));
    }

    #[test]
    fn a_corrupt_ledger_establishes_no_ownership() {
        let base = tempfile::tempdir().expect("temporary base directory");
        let root = base.path().join("docs/snippets");
        std::fs::create_dir_all(&root).expect("create snippet root");
        std::fs::write(root.join(COVERAGE_MANIFEST), b"{ not json").expect("write corrupt ledger");
        snapshot_pre_run_ledger(&root);

        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &root.join("python/chat/smoke.md")
        ));
    }

    /// Fail-closed: without a snapshot the guard's existing refusal stands unchanged, so a caller
    /// that never ran snippet generation cannot claim anything. ~keep
    #[test]
    fn nothing_is_claimable_without_a_snapshot() {
        let base = tempfile::tempdir().expect("temporary base directory");
        let root = base.path().join("docs/snippets");
        std::fs::create_dir_all(&root).expect("create snippet root");
        std::fs::write(
            root.join(COVERAGE_MANIFEST),
            serde_json::to_vec(&ledger(COVERAGE_MANIFEST_VERSION, &["python/chat/smoke.md"])).expect("serialize"),
        )
        .expect("write ledger");

        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &root.join("python/chat/smoke.md")
        ));
    }

    /// The snapshot is pinned on first read. A ledger rewritten mid-run — which is exactly what
    /// `e2e::run` does, ahead of the snippets, because `.alef-snippet-coverage.json` sorts before
    /// its sibling directories — must not widen what this run may claim. ~keep
    #[test]
    fn a_ledger_rewritten_after_the_snapshot_cannot_widen_ownership() {
        let (base, root) = snapshot(&ledger(COVERAGE_MANIFEST_VERSION, &["python/chat/smoke.md"]));
        std::fs::write(
            root.join(COVERAGE_MANIFEST),
            serde_json::to_vec(&ledger(
                COVERAGE_MANIFEST_VERSION,
                &["python/chat/smoke.md", "python/chat/newly-claimed.md"],
            ))
            .expect("serialize"),
        )
        .expect("rewrite ledger");
        snapshot_pre_run_ledger(&root);

        assert!(is_ledger_owned_snippet_path(
            base.path(),
            &root.join("python/chat/smoke.md")
        ));
        assert!(
            !is_ledger_owned_snippet_path(base.path(), &root.join("python/chat/newly-claimed.md")),
            "a path this run merely intends to write is not a path a previous run recorded"
        );
    }

    /// `generated_paths` is not consulted. It is the parallel array; `generated_metadata` is the
    /// field `coverage::orphaned_paths` trusts, and a ledger that disagrees between the two is
    /// rejected by `coverage::validate_generated_metadata` anyway — so an entry present only in
    /// `generated_paths` must not confer ownership. ~keep
    #[test]
    fn a_path_present_only_in_generated_paths_is_not_claimable() {
        let mut disagreeing = ledger(COVERAGE_MANIFEST_VERSION, &["python/chat/smoke.md"]);
        disagreeing
            .generated_paths
            .push(PathBuf::from("python/chat/metadata-free.md"));
        let (base, root) = snapshot(&disagreeing);

        assert!(is_ledger_owned_snippet_path(
            base.path(),
            &root.join("python/chat/smoke.md")
        ));
        assert!(!is_ledger_owned_snippet_path(
            base.path(),
            &root.join("python/chat/metadata-free.md")
        ));
    }
}
