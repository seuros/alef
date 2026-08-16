//! `alef adopt` — the only route by which a pre-existing file acquires alef ownership.
//!
//! The ownership guard in `cli::pipeline::generate::write` refuses to write any
//! pre-existing file it cannot prove it authored. That is correct, and it is also a
//! one-way door: a file whose type became stampable only *after* it was committed
//! carries no marker, so the write is refused, so the marker never lands, so the write
//! is refused forever. A consumer repo's `crates/<crate>-ffi/Cargo.toml` is in exactly
//! that state — `git log -S 'alef:hash'` over its entire history returns nothing — and
//! real fixes are frozen out of that repo behind a `warn!` nobody reads during a regen. That is
//! strictly worse than a create-once file: a create-once file is at least stable, while
//! this is a file alef believes it owns, intends to rewrite every run, and silently
//! declines to touch.
//!
//! This command is the door out, and it is deliberately narrow:
//!
//! - **Explicit and human-invoked.** One path or glob per invocation. It is not wired
//!   into `alef all`, `alef generate`, or any other command, and must not be.
//! - **Dry-run by default.** A bare `alef adopt <path>` prints the full diff and
//!   changes nothing; `--write` applies.
//! - **The full diff, never truncated, for every file whose content actually differs.**
//!   Unlike `alef migrate`'s preview, which caps at `MAX_DIFF_LINES` because a config
//!   migration is mechanical, adoption is a consent decision over content. A truncated
//!   diff is a diff the human did not read.
//! - **Adoption stamps the marker onto the bytes already on disk.** It never writes
//!   generated content. Convergence happens on the next ordinary `alef generate`,
//!   through the guard, where `git diff` shows it.
//!
//! What a diff is *for* splits the two states apart, and only one of them scales.
//! A [`AdoptionState::Drifted`] file's diff shows content adoption puts at risk, so it is
//! rendered in full and every route that could adopt one prints it first. A
//! [`AdoptionState::Converged`] file has no such content: its bytes already equal this
//! run's output apart from the marker, so its "diff" is the whole file echoed back as
//! context lines and reading 12,000 of them is not consent, it is noise that hides the
//! handful of real diffs in the same run. Converged files are therefore summarised by
//! count and adopted as a group, and `--converged-only` exists so a migration at that
//! scale can be performed by a command that is *incapable* of touching a drifted file.
//! Consent for the converged case still lives where it always did — a human typed the
//! glob and typed `--write` — and that has not been loosened. ~keep
//!
//! Why none of this can be automated: an automatic adoption of a drifted file is
//! byte-for-byte indistinguishable from clobbering a hand-edit — both are "regenerated
//! content replaces different existing content" — and an automatic adoption of a
//! *converged* file is indistinguishable from claiming a hand-written file that happens
//! to coincide, which is precisely the consumer-repo `e2e/go/helpers_test.go` incident
//! the guard was built for. The only thing separating the safe case from the unsafe one is a
//! human reading the diff. Automate it and the guard is deleted while the warning
//! remains. See `cli::pipeline::generate::write::stamp_for_adoption`. ~keep

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// How the bytes on disk relate to what alef would generate for the same path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionState {
    /// Already carries a provenance marker — the guard already permits writes, so
    /// there is nothing to adopt.
    AlreadyOwned,
    /// Identical to this run's output apart from the marker header. Adoption changes
    /// nothing a later generate would not already produce.
    Converged,
    /// The body genuinely differs. Adoption is consent for the next `alef generate` to
    /// replace this content, which is why it is never taken without a printed diff.
    Drifted,
}

/// One alef-managed output path, paired with the exact bytes the writer would put
/// there. Produced by [`managed_outputs`] so the diff a human reads is the writer's
/// real output, not an approximation of it.
pub struct ManagedOutput {
    pub relative: PathBuf,
    pub content: String,
}

/// A pre-existing file matched by the adopt target, classified against generated output.
pub struct AdoptCandidate {
    pub relative: PathBuf,
    pub full_path: PathBuf,
    pub existing: String,
    pub generated: String,
    pub state: AdoptionState,
    /// The exact bytes adoption would put on disk, or `None` when the format cannot
    /// carry a marker at all and ownership has to go through the committed record.
    ///
    /// Computed during classification rather than at apply time because it *is* the
    /// classification: [`AdoptionState::Converged`] is defined as "once stamped, the
    /// next generate is a byte no-op", which cannot be decided without the stamped
    /// bytes in hand. ~keep
    pub stamped: Option<String>,
}

pub struct AdoptOptions {
    /// A repo-relative path or glob, e.g. `crates/foo-ffi/Cargo.toml` or
    /// `packages/**/*.gemspec`.
    pub target: String,
    pub base_dir: PathBuf,
    /// `false` (the default) prints the diff and touches nothing.
    pub write: bool,
    /// Adopt only [`AdoptionState::Converged`] matches, leaving every drifted match
    /// untouched and reported.
    ///
    /// The bulk-migration switch, and deliberately *subtractive*: it can only ever
    /// adopt a strict subset of what a plain `--write` would. There is no additive
    /// counterpart — no `--all`, no `--yes` — because the thing such a flag would buy
    /// is skipping the drifted diffs, and a drifted file may be a deliberate hand-edit,
    /// which is the exact content the guard exists to protect. ~keep
    pub converged_only: bool,
}

/// The rendered diff for one candidate.
///
/// Carried in [`AdoptReport`] rather than only written to stdout so the diff step is
/// part of this module's contract and can be asserted on. A test that reads
/// `AdoptReport::diffs` fails if the diff stops being produced — which a test that only
/// asserted on the final file contents would not. ~keep
#[derive(Debug)]
pub struct AdoptDiff {
    pub relative: PathBuf,
    pub state: AdoptionState,
    pub body: String,
}

#[derive(Debug, Default)]
pub struct AdoptReport {
    /// Paths whose marker (or durable ownership record) was actually written.
    /// Always empty when `write` was false.
    pub adopted: Vec<PathBuf>,
    /// Paths that already carried a marker; no diff is produced for these.
    pub already_owned: Vec<PathBuf>,
    /// Matches whose bytes already equal this run's output apart from the marker.
    ///
    /// Reported as a set of paths rather than as [`Self::diffs`] entries: their diff
    /// is the file echoed back, so a per-file read buys the reader nothing and at
    /// consumer-repo scale actively buries the drifted diffs that do matter. ~keep
    pub converged: Vec<PathBuf>,
    /// Drifted matches left untouched because [`AdoptOptions::converged_only`] was set.
    pub skipped_drifted: Vec<PathBuf>,
    /// Paths adopted through the committed `.alef-ownership.toml` record because their
    /// format cannot carry a marker at all. Reported separately from [`Self::adopted`]
    /// because adopting these leaves the file itself byte-identical: the consent lives
    /// in a *different* file, which the operator has to commit for the adoption to mean
    /// anything anywhere else. ~keep
    pub recorded_unstampable: Vec<PathBuf>,
    /// Full, untruncated diffs for every drifted candidate, in path order.
    pub diffs: Vec<AdoptDiff>,
    /// True when this was a preview; nothing on disk was touched.
    pub preview: bool,
}

impl AdoptReport {
    pub fn drifted(&self) -> impl Iterator<Item = &AdoptDiff> + '_ {
        self.diffs.iter().filter(|diff| diff.state == AdoptionState::Drifted)
    }
}

/// Apply the writer's own normalization and header logic to a generated-file set,
/// yielding the exact bytes `write_files_report` / `write_scaffold_files_report` would
/// place on disk.
///
/// Routed through the same `normalize_content` + `ensure_generated_header` pair the
/// writers use rather than reimplemented, so the diff cannot drift from what a
/// subsequent `alef generate` actually does. A diff that is merely close is a diff that
/// obtained consent for something else. ~keep
pub fn managed_outputs(files: &[crate::core::backend::GeneratedFile], base_dir: &Path) -> Vec<ManagedOutput> {
    files
        .iter()
        .map(|file| {
            let full_path = base_dir.join(&file.path);
            let normalized = crate::cli::pipeline::normalize_content(&full_path, &file.content);
            let content = if file.generated_header {
                crate::cli::pipeline::ensure_generated_header(&full_path, &normalized)
            } else {
                normalized
            };
            ManagedOutput {
                relative: file.path.clone(),
                content,
            }
        })
        .collect()
}

/// Match `target` against a managed output path.
///
/// A literal path compares equal; anything else is a glob. `**` and `*` both cross
/// directory separators here, which is deliberate for a command a human types with a
/// specific tree in front of them — the safety of this command is the printed diff and
/// the `--write` gate, not the narrowness of the pattern.
fn matches_target(target: &str, relative: &Path) -> bool {
    let spelled = relative.to_string_lossy().replace('\\', "/");
    let target = target.trim_start_matches("./");
    if spelled == target {
        return true;
    }
    glob::Pattern::new(target).is_ok_and(|pattern| pattern.matches(&spelled))
}

/// The regenerate command embedded in a self-marking Markdown header, read back out
/// of the generated bytes.
///
/// `docs::render::with_html_header` writes its three comment lines contiguously and
/// takes the command as a parameter, so the value differs per producer (`alef docs`,
/// `alef readme`, `alef e2e generate`). Re-deriving it from the generated file rather
/// than guessing one is what makes a stamped file byte-identical to what the next
/// generate would emit — guess wrong and every snippet classifies as drifted on a
/// line the human never wrote. The marker line is located through
/// `content_has_alef_marker` itself rather than by re-typing the marker text. ~keep
fn embedded_regenerate_command(generated: &str) -> Option<String> {
    let lines: Vec<&str> = generated.lines().collect();
    let marker = lines
        .iter()
        .position(|line| crate::core::hash::content_has_alef_marker(line))?;
    let rest = lines
        .get(marker + 1)?
        .trim()
        .strip_prefix("<!--")?
        .trim()
        .strip_prefix("To regenerate:")?;
    Some(rest.trim().trim_end_matches("-->").trim().to_owned())
}

/// The bytes adoption would leave on disk, or `None` when nothing can be stamped and
/// ownership must go through the committed record instead.
///
/// Two routes, tried in order:
///
/// 1. [`crate::cli::pipeline::stamp_for_adoption`] — the generic path-driven header,
///    for every format `write::marker_header_syntax` knows a comment syntax for.
/// 2. **Self-marking Markdown**, driven by the *generated* content rather than the
///    path. `.md` is deliberately absent from `marker_header_syntax` (see its doc:
///    listing it would flip `.md` onto the ownership rail and retroactively freeze
///    every unmarked `.md` in every consumer repo), so route 1 refuses it — yet alef
///    plainly can mark Markdown, because every generated `.md` it emits already
///    carries an HTML-comment marker put there by `docs::render::with_html_header`,
///    and `write_scaffold_files_report`'s guard reads that marker on any extension.
///    Without this route the ~12k frozen snippet `.md` files would each take an entry
///    in `.alef-ownership.toml` instead: a 12,000-line committed manifest standing in
///    for a marker the file can perfectly well hold, on a path where the marker is
///    strictly better (it cannot be separated from the file it describes). ~keep
fn stamp_for(full_path: &Path, existing: &str, generated: &str) -> Option<String> {
    if let Some(stamped) = crate::cli::pipeline::stamp_for_adoption(full_path, existing) {
        return Some(stamped);
    }
    if full_path.extension().and_then(|extension| extension.to_str()) != Some("md") {
        return None;
    }
    if !crate::core::hash::content_has_alef_marker(generated) {
        return None;
    }
    let command = embedded_regenerate_command(generated)?;
    Some(crate::docs::with_html_header(existing.to_owned(), &command))
}

/// Classify one pre-existing file against the bytes alef would generate for it.
///
/// Convergence is decided on the **stamped** bytes, using the writers' own
/// unchanged-predicate (`strip_hash_line` on both sides, as in
/// `write_files_report` and `write_scaffold_files_report`), so `Converged` means
/// exactly one thing: after this adoption, the next ordinary `alef generate` writes
/// no byte of this file. Comparing the *unstamped* bytes instead — which is what
/// this did before self-marking formats were reachable — classifies every one of
/// the frozen e2e snippets as drifted, because the only difference between them and
/// generated output is the marker block adoption is about to add. That would have
/// demanded 12,000 individual diff reads for 12,000 files with nothing to read. ~keep
pub fn classify(full_path: &Path, relative: &Path, generated: &str, existing: &str) -> AdoptCandidate {
    let stamped = stamp_for(full_path, existing, generated);
    let state = if crate::core::hash::content_has_alef_marker(existing) {
        AdoptionState::AlreadyOwned
    } else if crate::core::hash::strip_hash_line(stamped.as_deref().unwrap_or(existing))
        == crate::core::hash::strip_hash_line(generated)
    {
        AdoptionState::Converged
    } else {
        AdoptionState::Drifted
    };
    AdoptCandidate {
        relative: relative.to_path_buf(),
        full_path: full_path.to_path_buf(),
        existing: existing.to_owned(),
        generated: generated.to_owned(),
        state,
        stamped,
    }
}

/// Render the complete line-by-line diff between what is on disk and what alef would
/// generate. Never truncated — see this module's header for why.
///
/// Called for drifted candidates only. A converged candidate has no divergence to
/// render, so its diff would be the whole file repeated as context lines.
pub fn render_diff(candidate: &AdoptCandidate) -> String {
    let spelled = candidate.relative.display();
    let mut body = format!("--- {spelled} (on disk)\n+++ {spelled} (alef generate output)\n");
    let diff = similar::TextDiff::from_lines(candidate.existing.as_str(), candidate.generated.as_str());
    for change in diff.iter_all_changes() {
        let prefix = match change.tag() {
            similar::ChangeTag::Delete => '-',
            similar::ChangeTag::Insert => '+',
            similar::ChangeTag::Equal => ' ',
        };
        body.push(prefix);
        body.push_str(change.value());
        if !change.value().ends_with('\n') {
            body.push('\n');
        }
    }
    body
}

/// Collect every managed output the target selects, refusing paths alef does not
/// generate and paths that do not exist yet.
fn collect_candidates(options: &AdoptOptions, managed: &[ManagedOutput]) -> Result<Vec<AdoptCandidate>> {
    let mut matched: Vec<&ManagedOutput> = managed
        .iter()
        .filter(|output| matches_target(&options.target, &output.relative))
        .collect();
    matched.sort_by(|left, right| left.relative.cmp(&right.relative));

    if matched.is_empty() {
        // Refusing an unmatched target is the property that keeps `alef adopt` from
        // being a general-purpose "stamp this file" tool: a path alef does not generate
        // can never be adopted, whatever the human types. ~keep
        bail!(
            "no alef-managed output matches `{}` -- adopt only applies to paths alef generates",
            options.target
        );
    }

    let mut candidates = Vec::with_capacity(matched.len());
    for output in matched {
        let full_path = options.base_dir.join(&output.relative);
        if !full_path.exists() {
            // Nothing to consent to: the guard never engages on a path with no
            // pre-existing content, so an ordinary generate already writes this. ~keep
            continue;
        }
        let existing = std::fs::read_to_string(&full_path)
            .with_context(|| format!("failed to read existing {}", full_path.display()))?;
        candidates.push(classify(&full_path, &output.relative, &output.content, &existing));
    }

    if candidates.is_empty() {
        bail!(
            "`{}` matches alef-managed output but nothing exists on disk yet -- \
             run `alef generate`, there is no ownership conflict to resolve",
            options.target
        );
    }
    Ok(candidates)
}

/// Stamp one candidate so a later run's ownership guard recognises it.
///
/// Writes the *existing* bytes plus a header, never the generated bytes. Formats with
/// no marker syntax fall back to the committed `.alef-ownership.toml` record, which is
/// the same proof route the guard already consults for them.
///
/// Both routes leave their proof inside the repository, which is what makes an adoption
/// mean the same thing on the machine that performed it and on a fresh clone of the
/// commit that captured it. This is the *only* way a path enters that record other than
/// alef creating the file itself — nothing infers ownership from content. ~keep
///
/// Unstampable paths are accumulated into `to_record` instead of being written one at
/// a time: `record_scaffold_owned_path` is a read-modify-write of the entire manifest,
/// so a per-path call inside this loop is O(n) parses of an O(n)-sized file. A single
/// batched [`crate::cli::cache::record_scaffold_owned_paths`] at the end of the run
/// makes a 12k-path migration linear. ~keep
fn apply(candidate: &AdoptCandidate, report: &mut AdoptReport, to_record: &mut Vec<PathBuf>) -> Result<()> {
    match &candidate.stamped {
        Some(stamped) => {
            crate::cli::pipeline::atomic_write(&candidate.full_path, stamped.as_bytes())?;
            crate::cli::pipeline::apply_shebang_chmod(&candidate.full_path, stamped)?;
            report.adopted.push(candidate.relative.clone());
        }
        None => {
            to_record.push(candidate.full_path.clone());
            report.recorded_unstampable.push(candidate.relative.clone());
            report.adopted.push(candidate.relative.clone());
        }
    }
    Ok(())
}

/// Run the adopt command against a pre-computed managed-output set.
///
/// `managed` is passed in rather than derived here so the whole command is exercisable
/// without a config, an extraction pass, or a real crate — the ownership decision is
/// the thing worth testing, and it must not be reachable only through a full pipeline.
pub fn run(options: &AdoptOptions, managed: &[ManagedOutput]) -> Result<AdoptReport> {
    let candidates = collect_candidates(options, managed)?;
    let mut report = AdoptReport {
        preview: !options.write,
        ..AdoptReport::default()
    };

    for candidate in &candidates {
        match candidate.state {
            AdoptionState::AlreadyOwned => report.already_owned.push(candidate.relative.clone()),
            AdoptionState::Converged => report.converged.push(candidate.relative.clone()),
            AdoptionState::Drifted => report.diffs.push(AdoptDiff {
                relative: candidate.relative.clone(),
                state: candidate.state,
                body: render_diff(candidate),
            }),
        }
    }

    for diff in report.drifted() {
        tracing::warn!(
            path = %diff.relative.display(),
            "content differs from generated output: adopting consents to alef replacing it on the next generate"
        );
    }

    if !options.write {
        return Ok(report);
    }

    let mut to_record: Vec<PathBuf> = Vec::new();
    for candidate in candidates.iter().filter(|c| c.state != AdoptionState::AlreadyOwned) {
        if options.converged_only && candidate.state == AdoptionState::Drifted {
            report.skipped_drifted.push(candidate.relative.clone());
            continue;
        }
        apply(candidate, &mut report, &mut to_record)?;
        if candidate.state == AdoptionState::Drifted {
            // Per-file at DEBUG for converged paths and INFO only for drifted ones: a
            // converged bulk adoption is one event of 12,000 paths, and emitting 12,000
            // INFO lines for it drowns the drifted adoptions, which are the ones a reader
            // has to be able to find in the log afterwards. ~keep
            tracing::info!(path = %candidate.relative.display(), "adopted (drifted): marker stamped, content kept");
        } else {
            tracing::debug!(path = %candidate.relative.display(), "adopted (converged): marker stamped");
        }
    }
    let record_refs: Vec<&Path> = to_record.iter().map(PathBuf::as_path).collect();
    crate::cli::cache::record_scaffold_owned_paths(&options.base_dir, &record_refs)?;
    Ok(report)
}

#[cfg(test)]
mod tests;
