//! Central, committed record of each crate's generation-inputs fingerprint, replacing the
//! design where that fingerprint was folded into every generated file's own `alef:hash:`
//! stamp. See `core::hash`'s module doc ("Hash semantics" and "Migration from v0.21.0 —
//! v0.71.x") for why: mixing a whole-crate fingerprint into a per-file value meant
//! any unrelated source or `alef.toml` change restamped every generated file, even ones whose
//! own emitted bytes never moved. Measured across real consumer repos, that was 98.8% of a
//! generated-file diff being a hash-only line change with no other content difference.
//!
//! This record answers the question the per-file stamp used to answer badly: "are this
//! crate's generation inputs (Rust sources + `alef.toml`) still what they were when it was
//! last generated?" It is written once per crate, by `alef generate`/`alef all`, after a
//! successful run — never per file, and never as a side effect of a narrower command (`alef
//! readme`, `alef docs`, `alef sync-versions`) that only touches a subset of a crate's
//! output. `alef verify` recomputes the current fingerprint and compares it to what is
//! recorded here via [`stale_crate_names`].
//!
//! Same committed-record shape as [`super::OWNERSHIP_MANIFEST`] and
//! `super::TOML_MERGE_PROVENANCE_MANIFEST` (repo root, `.alef-` reserved namespace, listed in
//! [`super::REQUIRED_COMMITTED_RECORDS`] so `alef verify` fails if it exists but is
//! untracked), but unlike [`super::OWNERSHIP_MANIFEST`] this record has no legacy-gitignored
//! read fallback and no cross-machine migration bridge: like
//! `super::TOML_MERGE_PROVENANCE_MANIFEST`, losing it is safe in only one direction —
//! [`stale_crate_names`] treats "no recorded value for this crate" as "no baseline to compare
//! against yet", not as a failure, so a repo that has never generated under this record's
//! format (every consumer, immediately after upgrading to this version) is silently skipped
//! rather than reported stale. The first `alef generate`/`alef all` after upgrading creates
//! it.
//!
//! This module also owns a second, deliberately UNcommitted record alongside the fingerprint
//! above: the per-crate in-progress marker ([`mark_generation_in_progress`] /
//! [`clear_generation_in_progress`] / [`generation_in_progress`]), which answers "did this
//! crate's most recent run finish" so an `alef all --clean` killed mid-run leaves a signal
//! `alef verify` can read instead of reporting the resulting absences as ordinary staleness
//! (alef#268). See [`mark_generation_in_progress`]'s doc for why it lives in the gitignored
//! `.alef/` cache rather than next to [`GENERATION_RECORD`] here. ~keep

use std::collections::BTreeMap;
use std::path::Path;

pub(super) const GENERATION_RECORD: &str = ".alef-generation.toml";

const GENERATION_RECORD_HEADER: &str = "\
# alef generation-inputs record -- COMMIT THIS FILE, do not add it to .gitignore.
#
# Records, per crate, the generation-inputs fingerprint (Rust sources + alef.toml -- see
# `core::hash::compute_inputs_hash`) as of that crate's most recent successful `alef
# generate`/`alef all` run. `alef verify` recomputes the current fingerprint and compares it
# to the value recorded here to detect a stale tree -- inputs changed since the last
# generation -- without folding that fingerprint into every generated file's own
# `alef:hash:` stamp, which used to restamp every file on any unrelated config or source
# change (see `core::hash`'s module doc for the incident this record replaces).
#
# Do not hand-edit; it is rewritten by `alef generate`/`alef all` on every successful run.
";

#[derive(Default, serde::Deserialize)]
struct GenerationRecordFile {
    #[serde(default)]
    crates: BTreeMap<String, String>,
}

fn generation_record_path(base_dir: &Path) -> std::path::PathBuf {
    base_dir.join(GENERATION_RECORD)
}

/// Read every recorded `(crate_name, inputs_hash)` pair, degrading a missing or unreadable
/// record to empty -- the safe direction for a query, mirroring
/// [`super::read_committed_owned_paths`]: a repo that has never generated under this record's
/// format, or whose record was hand-corrupted, must read as "no baseline" rather than error
/// out every `alef verify` run.
fn read_generation_record(base_dir: &Path) -> BTreeMap<String, String> {
    let Ok(content) = std::fs::read_to_string(generation_record_path(base_dir)) else {
        return BTreeMap::new();
    };
    match toml::from_str::<GenerationRecordFile>(&content) {
        Ok(record) => record.crates,
        Err(error) => {
            tracing::warn!(
                manifest = %GENERATION_RECORD,
                %error,
                "the alef generation-inputs record could not be parsed; treating every crate as \
                 having no recorded baseline until it is repaired"
            );
            BTreeMap::new()
        }
    }
}

/// The generation-inputs fingerprint recorded for `crate_name`'s most recent successful
/// generation, or `None` when there is no baseline yet -- the record is entirely absent, does
/// not mention this crate, or could not be parsed. All three cases are deliberately
/// indistinguishable to callers: see this module's doc for why "no baseline" must never be
/// treated as "stale".
pub fn recorded_inputs_hash(base_dir: &Path, crate_name: &str) -> Option<String> {
    read_generation_record(base_dir).get(crate_name).cloned()
}

/// Names, from `current`, of every crate whose freshly computed `inputs_hash` differs from
/// what is recorded for it -- i.e. a crate whose Rust sources or `alef.toml` have changed
/// since its last successful `alef generate`/`alef all` run. `current` is `(crate_name,
/// current_inputs_hash)` pairs; a crate with no recorded baseline is silently omitted, never
/// reported (see this module's doc). Reads the record once for the whole batch rather than
/// once per crate via [`recorded_inputs_hash`], so an N-crate workspace parses the file once,
/// not N times.
pub fn stale_crate_names<'a>(base_dir: &Path, current: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let recorded = read_generation_record(base_dir);
    current
        .into_iter()
        .filter_map(|(name, inputs_hash)| {
            let previous = recorded.get(name)?;
            (previous != inputs_hash).then(|| name.to_string())
        })
        .collect()
}

/// Render `crates` as one `name = "hash"` line per crate under a `[crates]` table, sorted
/// (via the `BTreeMap` iteration order) for a stable diff. Crate names are valid bare TOML
/// keys (cargo restricts them to `[A-Za-z0-9_-]`), so no key-escaping is needed; the value is
/// still rendered through `toml_edit::Value` rather than hand-interpolated, matching
/// [`super::render_record_assignment`]'s reasoning: a malformed value must never produce a
/// record that fails to parse.
fn render_generation_record(crates: &BTreeMap<String, String>) -> String {
    let mut body = String::from(GENERATION_RECORD_HEADER);
    body.push_str("\n[crates]\n");
    for (name, hash) in crates {
        body.push_str(name);
        body.push_str(" = ");
        body.push_str(&toml_edit::Value::from(hash.as_str()).to_string());
        body.push('\n');
    }
    body
}

/// Record `crate_name`'s current generation-inputs fingerprint in the committed
/// [`GENERATION_RECORD`], replacing any previous entry for the same crate. Called once per
/// crate at the end of a successful `alef generate`/`alef all` run -- see this module's doc
/// for why it is not folded into `cli::pipeline::generate::write::finalize_hashes`, which has
/// no crate name in scope and runs many times per crate per command.
///
/// A no-op, byte-for-byte, when the recorded value already matches -- so a converged tree
/// never rewrites the file and never produces a spurious diff, matching every other committed
/// record in this module.
pub fn record_inputs_hash(base_dir: &Path, crate_name: &str, inputs_hash: &str) -> anyhow::Result<()> {
    // Serialised for the same reason `record_scaffold_owned_paths` is: a read-modify-write of
    // one file, called once per crate in a workspace loop that could in principle run
    // concurrently in the future. ~keep
    static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());

    let mut crates = read_generation_record(base_dir);
    if crates.get(crate_name).map(String::as_str) == Some(inputs_hash) {
        return Ok(());
    }

    std::fs::create_dir_all(base_dir)?;
    let is_new_record = !generation_record_path(base_dir).is_file();
    crates.insert(crate_name.to_string(), inputs_hash.to_string());
    std::fs::write(generation_record_path(base_dir), render_generation_record(&crates))?;

    if is_new_record {
        tracing::info!(
            manifest = %GENERATION_RECORD,
            "created the alef generation-inputs record: commit it, or a fresh clone/CI cannot \
             detect a stale tree until the next generate"
        );
        super::rearm_untracked_record_notice(base_dir);
    }
    super::note_untracked_required_records(base_dir);
    Ok(())
}

/// File name of the per-crate "a generation run started and has not yet finished" marker.
/// Lives under `.alef/<crate_name>/`, never at the repo root next to [`GENERATION_RECORD`].
///
/// This is deliberately the gitignored `.alef/` cache, not a committed record, and that is the
/// load-bearing decision in this module (see alef#268). [`GENERATION_RECORD`] answers "what was
/// this crate's generation-inputs fingerprint as of its last SUCCESSFUL run" and is committed
/// because every checkout must agree on that baseline. This marker answers a different, purely
/// local question: "does a run against *this working tree, right now* need to be treated as
/// having died mid-flight". A fresh clone has no `.alef/` at all, so it can never inherit a stale
/// in-progress marker from a machine or CI run that happened to be killed -- the marker starts
/// every checkout in the same "no run in flight" state a clean tree is already in. Committing it
/// would invert that: a marker written before an interrupted run and accidentally committed
/// would tell every future clone, forever, that generation never finished, with no run left to
/// clear it. See `read_before_write`/`verify-before-acting`-flavoured reasoning in `cache.rs`'s
/// `OWNERSHIP_MANIFEST` doc for why the *other* records in this family choose the opposite
/// answer -- they encode provenance a fresh clone must inherit; this encodes in-flight process
/// state a fresh clone must never inherit. ~keep
const GENERATION_IN_PROGRESS_MARKER: &str = "generation-in-progress";

fn generation_in_progress_marker_path(base_dir: &Path, crate_name: &str) -> std::path::PathBuf {
    base_dir
        .join(super::CACHE_DIR)
        .join(crate_name)
        .join(GENERATION_IN_PROGRESS_MARKER)
}

/// Content of the marker file -- a note for a human who opens it directly, not a format any
/// caller parses. See [`generation_in_progress`]: existence alone is the whole signal. ~keep
const GENERATION_IN_PROGRESS_MARKER_CONTENT: &str = "\
alef generation in progress -- if this file is still here, the run that created it did not
finish; rerun `alef all`/`alef generate` for this crate.
";

/// Record that a generation run for `crate_name` has started but not yet completed.
///
/// Callers must call this once per crate, before the first mutation that crate's run makes
/// (before `--clean`'s removal, before any file write) -- see [`clear_generation_in_progress`]
/// for the other half. A plain file write, not a `Drop` guard or a signal handler: the whole
/// point is to survive the process being KILLED outright, which runs neither. ~keep
pub fn mark_generation_in_progress(base_dir: &Path, crate_name: &str) -> anyhow::Result<()> {
    super::validate_cache_crate_name(crate_name)?;
    let path = generation_in_progress_marker_path(base_dir, crate_name);
    if let Some(parent) = path.parent() {
        crate::core::cache_dir::ensure_cache_dir(parent)?;
    }
    std::fs::write(path, GENERATION_IN_PROGRESS_MARKER_CONTENT)?;
    Ok(())
}

/// Clear the marker [`mark_generation_in_progress`] wrote, once `crate_name`'s run has
/// completed successfully. Idempotent: clearing an already-absent marker is not an error, so a
/// caller never has to special-case "this is the first run for this crate".
pub fn clear_generation_in_progress(base_dir: &Path, crate_name: &str) -> anyhow::Result<()> {
    match std::fs::remove_file(generation_in_progress_marker_path(base_dir, crate_name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Whether `crate_name`'s most recent generation run started but never completed, per
/// [`mark_generation_in_progress`]/[`clear_generation_in_progress`].
pub fn generation_in_progress(base_dir: &Path, crate_name: &str) -> bool {
    generation_in_progress_marker_path(base_dir, crate_name).is_file()
}

/// Names, from `crate_names`, of every crate whose last generation run did not complete --
/// the batch form of [`generation_in_progress`], mirroring [`stale_crate_names`]'s shape so
/// `alef verify` can compute both in the same style. ~keep
pub fn incomplete_crate_names<'a>(base_dir: &Path, crate_names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    crate_names
        .into_iter()
        .filter(|name| generation_in_progress(base_dir, name))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_inputs_hash_is_none_when_the_record_is_entirely_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(recorded_inputs_hash(dir.path(), "my-crate"), None);
    }

    #[test]
    fn recorded_inputs_hash_is_none_for_a_crate_the_record_never_mentions() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "other-crate", "hash-a").expect("record other-crate");
        assert_eq!(recorded_inputs_hash(dir.path(), "my-crate"), None);
    }

    #[test]
    fn record_inputs_hash_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "my-crate", "hash-a").expect("record");
        assert_eq!(recorded_inputs_hash(dir.path(), "my-crate"), Some("hash-a".to_string()));
    }

    #[test]
    fn record_inputs_hash_overwrites_a_previous_value_for_the_same_crate() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "my-crate", "hash-a").expect("first record");
        record_inputs_hash(dir.path(), "my-crate", "hash-b").expect("second record");
        assert_eq!(recorded_inputs_hash(dir.path(), "my-crate"), Some("hash-b".to_string()));
    }

    #[test]
    fn record_inputs_hash_keeps_a_sibling_crates_entry_in_a_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "crate-a", "hash-a").expect("record crate-a");
        record_inputs_hash(dir.path(), "crate-b", "hash-b").expect("record crate-b");
        assert_eq!(recorded_inputs_hash(dir.path(), "crate-a"), Some("hash-a".to_string()));
        assert_eq!(recorded_inputs_hash(dir.path(), "crate-b"), Some("hash-b".to_string()));
    }

    #[test]
    fn record_inputs_hash_is_a_no_op_when_the_value_is_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "my-crate", "hash-a").expect("first record");
        let before = std::fs::read_to_string(generation_record_path(dir.path())).expect("read record");
        record_inputs_hash(dir.path(), "my-crate", "hash-a").expect("second, unchanged record");
        let after = std::fs::read_to_string(generation_record_path(dir.path())).expect("read record");
        assert_eq!(
            before, after,
            "recording the same value again must not rewrite the file"
        );
    }

    /// Guarantee 2 (the stale-tree signal): once a crate has a recorded baseline, a current
    /// `inputs_hash` that no longer matches it must be reported. Revert this and every
    /// consumer's `alef verify` would silently accept a tree whose sources or `alef.toml`
    /// changed since the last generate. ~keep
    #[test]
    fn stale_crate_names_reports_a_crate_whose_current_inputs_hash_moved() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "my-crate", "hash-old").expect("record baseline");

        let stale = stale_crate_names(dir.path(), [("my-crate", "hash-new")]);

        assert_eq!(stale, vec!["my-crate".to_string()]);
    }

    #[test]
    fn stale_crate_names_is_silent_when_the_current_hash_still_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "my-crate", "hash-a").expect("record baseline");

        let stale = stale_crate_names(dir.path(), [("my-crate", "hash-a")]);

        assert!(stale.is_empty());
    }

    /// The migration-graceful half of guarantee 2: a crate with no recorded baseline yet --
    /// every crate in every consumer repo immediately after upgrading to this version, before
    /// the first `alef generate` runs under the new record -- must not be reported stale.
    /// Getting this wrong would make `alef verify` fail on a signal it invented, on top of the
    /// expected one-time per-file re-stamp `core::hash`'s module doc already accounts for. ~keep
    #[test]
    fn stale_crate_names_does_not_report_a_crate_with_no_recorded_baseline() {
        let dir = tempfile::tempdir().expect("tempdir");

        let stale = stale_crate_names(dir.path(), [("my-crate", "hash-anything")]);

        assert!(stale.is_empty());
    }

    #[test]
    fn stale_crate_names_only_reports_the_crates_that_actually_moved_in_a_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        record_inputs_hash(dir.path(), "stable-crate", "hash-stable").expect("record stable-crate");
        record_inputs_hash(dir.path(), "moved-crate", "hash-old").expect("record moved-crate");

        let stale = stale_crate_names(
            dir.path(),
            [("stable-crate", "hash-stable"), ("moved-crate", "hash-new")],
        );

        assert_eq!(stale, vec!["moved-crate".to_string()]);
    }

    /// Positive control for the in-progress marker: a crate nobody has ever marked, or one that
    /// was marked and then cleared (the ordinary successful-run shape), must read as complete.
    /// Without this the "reports incomplete" tests below would not prove the check fired --
    /// it could report every crate incomplete unconditionally and still pass them. ~keep
    #[test]
    fn generation_in_progress_is_false_for_a_crate_never_marked() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!generation_in_progress(dir.path(), "my-crate"));
    }

    #[test]
    fn mark_then_clear_returns_to_not_in_progress() {
        let dir = tempfile::tempdir().expect("tempdir");
        mark_generation_in_progress(dir.path(), "my-crate").expect("mark");
        assert!(generation_in_progress(dir.path(), "my-crate"));

        clear_generation_in_progress(dir.path(), "my-crate").expect("clear");
        assert!(
            !generation_in_progress(dir.path(), "my-crate"),
            "a crate whose run completed must be indistinguishable from one never marked"
        );
    }

    /// Simulates a process killed after `mark_generation_in_progress` but before
    /// `clear_generation_in_progress` ever ran -- the exact failure mode alef#268 describes.
    /// No `clear` call happens in this test on purpose. ~keep
    #[test]
    fn generation_in_progress_survives_as_true_when_never_cleared() {
        let dir = tempfile::tempdir().expect("tempdir");
        mark_generation_in_progress(dir.path(), "my-crate").expect("mark");
        assert!(generation_in_progress(dir.path(), "my-crate"));
    }

    #[test]
    fn clear_generation_in_progress_is_a_no_op_when_no_marker_was_ever_written() {
        let dir = tempfile::tempdir().expect("tempdir");
        clear_generation_in_progress(dir.path(), "my-crate").expect("clearing an absent marker must not error");
    }

    #[test]
    fn incomplete_crate_names_reports_only_the_marked_crate_in_a_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        mark_generation_in_progress(dir.path(), "interrupted-crate").expect("mark interrupted-crate");

        let incomplete = incomplete_crate_names(dir.path(), ["stable-crate", "interrupted-crate"]);

        assert_eq!(incomplete, vec!["interrupted-crate".to_string()]);
    }

    #[test]
    fn incomplete_crate_names_is_empty_once_every_marked_crate_is_cleared() {
        let dir = tempfile::tempdir().expect("tempdir");
        mark_generation_in_progress(dir.path(), "my-crate").expect("mark");
        clear_generation_in_progress(dir.path(), "my-crate").expect("clear");

        let incomplete = incomplete_crate_names(dir.path(), ["my-crate"]);

        assert!(incomplete.is_empty());
    }
}
