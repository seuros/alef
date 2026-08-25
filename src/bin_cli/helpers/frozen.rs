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
    /// The literal marker line `alef adopt` would add to the top of the file, or `None` when
    /// the format has no comment syntax to carry one (`.json`, lockfiles).
    ///
    /// Informational only -- [`report_lines`] never instructs a reader to paste this in by
    /// hand. `crate::cli::pipeline::generate::write::report_refused_writes` (the write guard's
    /// own refusal message) is explicit that hand-adding the marker is unsafe: "a refusal can
    /// be protecting a deliberate hand-edit, and stamping it blind re-enables exactly the
    /// clobbering the guard exists to prevent." `alef verify`'s report used to say the opposite
    /// -- "add the marker shown" -- which pointed a reader at the one workflow alef's own write
    /// guard warns against, instead of at `alef adopt`, the reviewed, diffed path that exists
    /// for exactly this. ~keep
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
    /// single time.
    ///
    /// Splitting the heading was not enough, and [`report_lines`] finished the job: a
    /// create-once seed is no longer reported as frozen at all, because the only remedy alef
    /// has for one is the flag its own output calls DANGEROUS, for a file alef never rewrites.
    /// The field survives because the count still has to be stated -- as coverage, in
    /// `bin_cli::verify_coverage`. ~keep
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

/// Whether any frozen file is one `alef adopt --write` will actually ACCEPT.
///
/// A create-once seed is excluded on purpose. Its missing marker is deliberate, not drift: the
/// write guard refuses it by design, a plain `alef generate` leaves it untouched, and adopting it
/// requires the explicit `--clobber-create-once-seeds`. Gating `alef verify`'s exit code on the
/// whole frozen list therefore made verify unable to reach exit 0 on any repo carrying legacy
/// pre-marker files -- no amount of regeneration cleared them -- so the release gate could only be
/// satisfied by reaching for a destructive flag. `create_once` comes from
/// [`crate::cli::commands::adopt::is_create_once_seed`], the identical predicate `alef adopt` gates
/// that flag on, so this and that refusal cannot drift apart. ~keep
pub(crate) fn has_adoptable_frozen_files(frozen: &[FrozenFile]) -> bool {
    frozen.iter().any(|file| !file.create_once)
}

/// The paths of every create-once seed on disk that carries no provenance marker.
///
/// Named as a *coverage* fact, not a finding: `alef verify` proves nothing about these files'
/// contents, and there is no action that changes that. See [`report_lines`] for why they are
/// no longer reported as frozen. ~keep
pub(crate) fn unmarked_create_once_seeds(frozen: &[FrozenFile]) -> Vec<&str> {
    frozen
        .iter()
        .filter(|file| file.create_once)
        .map(|file| file.path.as_str())
        .collect()
}

/// `alef verify`'s frozen-file report -- the ADOPTABLE entries only, one line each plus its
/// remedy.
///
/// A create-once seed is not reported here at all, and that is the fix rather than an
/// omission. [`FrozenFile`] means "alef would write this path and the guard refuses it
/// forever", and for a create-once seed the antecedent is false: alef emits the path only when
/// it is absent, so on an existing file there is no write to refuse and nothing is lost by the
/// missing marker. Reporting it as frozen described the file as a problem and then offered the
/// only escape alef has -- `alef adopt --write --clobber-create-once-seeds`, whose own output
/// calls it DANGEROUS -- for a file this repository's own documentation calls user-owned after
/// scaffold (`generated_header: false`). Measured in a consumer repo: `alef adopt
/// --converged-only` adopted 0 of 102 reported paths, 72 of them refused by alef itself as
/// seeds, including 13 LICENSE files and several `.gitkeep`s. A file cannot be both user-owned
/// and a verify finding.
///
/// The alternative -- recording ownership of a seed without touching its body -- was
/// considered and rejected: it buys no verification (alef still never rewrites the body, and
/// the stamp covers generation inputs rather than the seed's hand-grown contents) while handing
/// the write guard a licence it deliberately withholds, which is the exact protection
/// `--clobber-create-once-seeds` exists to gate.
///
/// The count does not disappear with the heading. `alef verify` states it in its coverage
/// report on every run, including a clean one, so these files move from "reported as a problem
/// only when something else already failed" to "always visible as unchecked" -- see
/// [`unmarked_create_once_seeds`] and `bin_cli::verify_coverage`. ~keep
pub(crate) fn report_lines(frozen: &[FrozenFile]) -> Vec<String> {
    let adoptable: Vec<&FrozenFile> = frozen.iter().filter(|file| !file.create_once).collect();
    if adoptable.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        "Frozen generated files detected (alef owns these paths but the files carry no provenance \
         marker, so alef refuses to write them -- review each file, then either run `alef adopt \
         <path> --write` to take ownership, or delete the file so generation can write it cleanly. \
         Never hand-add the marker line: alef's own write guard treats that as re-enabling exactly \
         the clobbering it exists to prevent -- see \
         `crate::cli::pipeline::generate::write::report_refused_writes`):"
            .to_owned(),
    ];
    for file in adoptable {
        lines.push(format!("  {}", file.path));
        if let Some(near_miss) = &file.near_miss {
            lines.push(format!(
                "    close but not recognized: {near_miss:?} (alef accepts \"generated by alef\" \
                 case-insensitively)"
            ));
        }
        lines.push(match &file.remedy {
            Some(remedy) => format!(
                "    run `alef adopt <path> --write` to add it (marker `alef adopt` would write: \
                 {remedy})"
            ),
            None => "    this format has no comment syntax to carry a marker, so alef proves ownership \
                     through the committed .alef-ownership.toml record instead -- run `alef adopt <path> \
                     --write` to record it there, or delete the file so the next `alef generate` writes \
                     and records it directly"
                .to_owned(),
        });
    }
    lines
}
