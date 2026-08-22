//! Frozen managed files -- pre-existing alef-owned paths that carry no provenance marker,
//! and so are deadlocked out of the write guard forever (see [`FrozenFile`]'s doc).
//!
//! Split out of `helpers.rs` rather than added to it: that file sits at this repository's
//! 1,000-line cap, and this concern -- deciding which pre-existing files are frozen, and
//! whether each one is a create-once seed or a genuinely adoptable frozen file -- is
//! self-contained enough to own its own module. ~keep

#[cfg(test)]
mod tests;

/// A generated file alef would own and mark, that already exists on disk but
/// carries no provenance marker at all.
///
/// This is a different, unrecoverable condition from a stale [`super::StaleMismatch`]
/// or a [`super::missing_managed_paths`] entry: the write guard in
/// `crate::cli::pipeline::generate::write::write_files_report` and
/// `crate::cli::pipeline::generate::scaffold::write_scaffold_files_report`
/// refuses to touch a pre-existing file that carries no marker (it cannot tell
/// a hand-written file from an alef output that predates the marker system),
/// and the marker can only ever be added *by* a write the guard has already
/// authorised — so an unmarked pre-existing file is frozen forever. Running
/// `alef generate` again does nothing; a human must read the file, then either
/// adopt it (paste `remedy` in and rerun `alef generate`) or delete it so
/// generation can write it cleanly. ~keep
pub(crate) struct FrozenFile {
    pub(crate) path: String,
    /// The literal marker line to add to the top of the file, or `None` when
    /// the format has no comment syntax to carry one (`.json`, lockfiles).
    pub(crate) remedy: Option<String>,
    /// A leading line in the existing file that looks like a failed attempt at a marker --
    /// see [`crate::core::hash::near_miss_marker`] -- so the report can point at what's already
    /// there instead of only showing what should be there. `None` when the file's leading lines
    /// don't mention alef and generation at all (a plain hand-written file). ~keep
    pub(crate) near_miss: Option<String>,
    /// Whether this path is a create-once seed under
    /// [`crate::cli::commands::adopt::is_create_once_seed`] -- the exact predicate `alef
    /// adopt` gates `--clobber-create-once-seeds` on, called here rather than
    /// re-derived, so the report and the command it points at can never disagree about
    /// which paths are seeds.
    ///
    /// THE DEFECT this closes: before this field existed, every frozen path was reported
    /// under one heading with one remedy ("run `alef adopt <path>`"), regardless of
    /// whether the path was a create-once seed. `alef adopt --write` then refused every
    /// seed outright -- measured at 85 of 85 in one consumer repo and 99 of 99 (all of
    /// them seeds) in another -- naming a flag (`--clobber-create-once-seeds`) the
    /// report never mentioned. A human followed the printed remedy and hit a wall every
    /// single time. ~keep
    pub(crate) create_once: bool,
}

/// [`FrozenFile`] entries for every alef-owned file in `files` that already
/// exists on disk but carries no marker.
///
/// Uses the same ownership predicate as [`super::missing_managed_paths`] — a
/// scaffold-once file alef never marks is excluded here exactly as it is from
/// the missing-file check, so a hand-edited `Cargo.toml`/`package.json`
/// template is never mistaken for a frozen generated file.
///
/// For a format [`crate::cli::pipeline::marker_comment_style`] has no comment syntax for
/// (`.json`, `DESCRIPTION`, a pre-widening `.clang-format`), a missing marker is not by
/// itself evidence of foreign authorship — [`crate::cli::pipeline::is_owned_by_ownership_record`]
/// is consulted exactly as `write_files_report`'s and `write_scaffold_files_report`'s write
/// guards consult it, so this report agrees with what those guards would actually accept.
/// Before this fell back to the marker check alone, a file the write guard would happily
/// (re)write on the strength of its committed `.alef-ownership.toml` record — including one
/// `alef adopt` or a delete-and-regenerate had just recorded — stayed reported "frozen"
/// forever, because this function never looked at the record the guard relies on. ~keep
///
/// The remedy text is read straight from the in-memory `GeneratedFile::content`
/// first, because a self-marking backend (custom Swift/Kotlin/Dart/Gleam/Zig
/// headers, `docs::render`'s HTML-commented `.md` pages) already bakes its
/// literal header into `content` regardless of `generated_header`. Only when
/// that content carries no marker yet — the common case, where the header is
/// added later by `write_files_report`'s `ensure_generated_header` pass — does
/// this fall back to reconstructing it from the path via
/// [`crate::cli::pipeline::provenance_header_for_path`]. ~keep
///
/// Runs over two candidate sets, not only [`crate::cli::pipeline::managed_generated_files`]'s
/// marker-carrying subset: `carries_alef_marker()` is `generated_header ||
/// content_has_alef_marker`, so a file emitted with `generated_header: false` whose content
/// embeds no marker at all — the PHP backend's `config.m4`
/// (`backends::php::gen_bindings::rust_items::generate_config_m4`) is the shipped case —
/// never reaches the ownership-record fallback a few lines below, even though
/// `write_files_report`'s guard already refuses to overwrite that exact path once it exists
/// without a committed `.alef-ownership.toml` record. [`unmarkable_unclaimed_files`] recovers
/// that second set: it is deliberately narrower than "every `generated_header: false` file" —
/// see its own doc for why only the genuinely unmarkable ones qualify.
///
/// `create_once` is computed from the *original* [`crate::core::backend::GeneratedFile`]
/// via [`crate::cli::commands::adopt::is_create_once_seed`] before it is consumed to build
/// `remedy`/`near_miss` below — the same predicate answers correctly for both candidate
/// sets without branching: every entry from `managed_generated_files` carries a marker
/// (`carries_alef_marker() == true`), which `is_create_once_seed` always answers `false`
/// for, so only entries recovered from `unmarkable_unclaimed_files` can ever be seeds. ~keep
pub(super) fn frozen_managed_paths(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
) -> Vec<FrozenFile> {
    crate::cli::pipeline::managed_generated_files(files)
        .into_iter()
        .chain(unmarkable_unclaimed_files(files, base_dir))
        .filter_map(|file| {
            let full_path = base_dir.join(&file.path);
            let existing = std::fs::read_to_string(&full_path).ok()?;
            if crate::core::hash::content_has_alef_marker(&existing) {
                return None;
            }
            let is_markable = crate::cli::pipeline::marker_comment_style(&full_path).is_some();
            if !is_markable && crate::cli::pipeline::is_owned_by_ownership_record(base_dir, &full_path) {
                return None;
            }
            let create_once = crate::cli::commands::adopt::is_create_once_seed(&file);
            let remedy = super::marker_line(&file.content).map(str::to_owned).or_else(|| {
                let header = crate::cli::pipeline::provenance_header_for_path(&file.path)?;
                super::marker_line(&header).map(str::to_owned)
            });
            let near_miss = crate::core::hash::near_miss_marker(&existing).map(str::to_owned);
            Some(FrozenFile {
                path: full_path.display().to_string(),
                remedy,
                near_miss,
                create_once,
            })
        })
        .collect()
}

/// Every file in `files` that [`crate::cli::pipeline::managed_generated_files`] excludes
/// (`carries_alef_marker()` is false — no `generated_header: true` claim, no marker baked
/// into `content`) but that is genuinely incapable of ever carrying one
/// ([`crate::cli::pipeline::marker_comment_style`] answers `None` for its path).
///
/// Scoped this narrowly on purpose: widening it to every `generated_header: false` file
/// would also pull in a markable file a backend simply forgot to self-mark, which
/// `write_files_report`'s guard treats differently — a markable path with no marker is
/// refused regardless of any ownership record (see that function's `owned` computation),
/// so folding it into this ownership-record-checked set would wrongly clear it once a
/// record existed. Only the genuinely unmarkable subset is where alef's write guard has
/// ever accepted an ownership record as proof, and this mirrors exactly that. ~keep
fn unmarkable_unclaimed_files(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
) -> Vec<crate::core::backend::GeneratedFile> {
    files
        .iter()
        .filter(|file| {
            !file.carries_alef_marker()
                && crate::cli::pipeline::marker_comment_style(&base_dir.join(&file.path)).is_none()
        })
        .cloned()
        .collect()
}
