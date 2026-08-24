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
//! - **Explicit and human-invoked.** One or more paths or globs per invocation, each
//!   resolved and reported independently (see `bin_cli::aux_commands`'s `Commands::Adopt`,
//!   which loops this module's single-target [`run`] once per requested target). It is
//!   not wired into `alef all`, `alef generate`, or any other command, and must not be.
//! - **Dry-run by default.** A bare `alef adopt <path>` prints the full diff and
//!   changes nothing; `--write` applies.
//! - **The full diff, never truncated, for every file whose content actually differs.**
//!   Unlike `alef migrate`'s preview, which caps at `MAX_DIFF_LINES` because a config
//!   migration is mechanical, adoption is a consent decision over content. A truncated
//!   diff is a diff the human did not read.
//! - **Adoption stamps the marker onto the bytes already on disk.** It never writes
//!   generated content. Convergence happens on the next ordinary `alef generate`,
//!   through the guard, where `git diff` shows it.
//! - **Create-once seeds are excluded from `--write` unless
//!   `--clobber-create-once-seeds` is passed**, and every excluded path is printed.
//!   Adoption means opposite things for the two rails, and only one of them is safe. On
//!   the marker rail alef rewrites the path every run, so unfreezing it is the point of
//!   this command. A seed is the reverse: alef emits it *once*, as a placeholder, and
//!   never revisits it, so whatever is on disk is by construction the grown-up version of
//!   that placeholder — a real 12-test suite where the seed is a three-line stub. For
//!   those paths the missing marker is not a bug to be fixed, it *is* the protection, and
//!   it is the only protection, because `write_scaffold_files_report`'s create-once skip
//!   is bypassed under `overwrite: true` — which a routine `alef version` bump passes
//!   (`version_regen::regenerate_scaffold_after_sync`). A repo-wide
//!   `alef adopt --write 'packages/**'` therefore arms the next version bump to replace
//!   real test suites with stubs, and the damage lands on a later, unrelated command
//!   where no diff review can catch it. Hence a separate, deliberately ugly flag rather
//!   than a wider `--write`. ~keep
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
    /// True when alef emits this path **only if it is absent** — a create-once seed.
    ///
    /// Computed by [`is_create_once_seed`] from
    /// [`crate::core::backend::GeneratedFile::carries_alef_marker`] (the flag or a
    /// self-marker in the content), never from sniffing the processed `content` above.
    /// Sniffing would misclassify every `generated_header: true` path whose format
    /// cannot hold a marker (`.json`, `.jar`, `DESCRIPTION`) as a seed, when those are
    /// squarely on the regeneration rail and prove ownership through the committed
    /// record instead. The one deliberate carve-out from the marker check is
    /// [`crate::cli::cache::is_alef_derived_output`] — artifacts the marker check alone
    /// cannot distinguish from a human-grown seed — see [`is_create_once_seed`]. ~keep
    pub create_once: bool,
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
    /// Mirrors [`ManagedOutput::create_once`] — alef emits this path only when absent.
    pub create_once: bool,
    /// Present when this candidate's bytes are not text ([`BinaryFacts`]); `existing`
    /// and `generated` are then empty and carry no meaning.
    pub binary: Option<BinaryFacts>,
}

/// What can be reviewed about a binary candidate, standing in for the line diff a text
/// candidate gets.
///
/// Adoption of a drifted path is only ever taken after the operator reads what is at
/// risk, and for bytes there is nothing to read line by line — which is why these paths
/// were previously refused outright. Refusal was the wrong conclusion: alef's *writers*
/// already own binaries, guarding them with `is_scaffold_owned_path` and recording them
/// with `record_scaffold_owned_path` (see `generate::scaffold::write_scaffold_files_report`
/// and `generate::write::write_files_report`). The ownership rail for binary output
/// exists and is load-bearing; the only thing missing was a door into it for a file alef
/// did not itself create, leaving such a path refused by the guard forever with no
/// command able to fix it. Size and digest on both sides are the reviewable unit that
/// bytes actually have, and they are enough to answer the question adoption asks: is the
/// artifact on disk the one alef would put there, or a different one you are consenting
/// to lose? ~keep
#[derive(Debug, Clone)]
pub struct BinaryFacts {
    pub existing_len: usize,
    pub existing_digest: String,
    pub generated_len: usize,
    pub generated_digest: String,
}

impl BinaryFacts {
    fn new(existing: &[u8], generated: &[u8]) -> Self {
        Self {
            existing_len: existing.len(),
            existing_digest: crate::core::hash::hash_bytes(existing),
            generated_len: generated.len(),
            generated_digest: crate::core::hash::hash_bytes(generated),
        }
    }
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
    /// adopt a strict subset of what a plain `--write` would. Nothing additive exists on
    /// *this* axis — no `--all`, no `--yes` — because the thing such a flag would buy is
    /// skipping the drifted diffs, and a drifted file may be a deliberate hand-edit,
    /// which is the exact content the guard exists to protect.
    ///
    /// [`Self::clobber_create_once_seeds`] is additive but on an orthogonal axis: it
    /// widens which *paths* are eligible and never suppresses a diff. ~keep
    pub converged_only: bool,
    /// Include create-once seeds ([`ManagedOutput::create_once`]) in the adoption.
    ///
    /// Off by default because adoption means the opposite thing for a seed than it does
    /// for a regenerated file, in the one direction that destroys work. For a file on the
    /// marker rail, adoption unfreezes a path alef rewrites every run — the whole point
    /// of the command. For a seed, alef's generated content is a *placeholder* it emits
    /// once and never revisits, so the on-disk file is by construction the grown-up
    /// version: a real test suite where the seed is three lines of stub. Adoption stamps
    /// the marker, and the marker is the only thing standing between that suite and the
    /// stub, because `write_scaffold_files_report`'s create-once skip (`can_skip`) is
    /// bypassed under `overwrite: true` — which `version_regen::regenerate_scaffold_after_sync`
    /// passes on a routine `alef version` bump. So a repo-wide `alef adopt --write` over a
    /// glob that happens to sweep in `packages/zig/test/*_test.zig` arms the next version
    /// bump to replace a hand-grown suite with a placeholder.
    ///
    /// That destruction is unrecoverable by review: the consent is recorded now and the
    /// overwrite happens on a later, unrelated command, far from the mistake. Hence a
    /// separate opt-in rather than a wider `--write`. ~keep
    pub clobber_create_once_seeds: bool,
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
    /// Create-once seeds excluded because [`AdoptOptions::clobber_create_once_seeds`] was
    /// not set, in path order.
    ///
    /// A `Vec` of paths rather than a count, and reported even in preview: these are the
    /// matches where adoption is destructive, so the operator has to be able to read the
    /// list and recognise a file as their own before deciding. A count tells them only
    /// that something was excluded, which is exactly the fact that does not help. ~keep
    pub skipped_create_once: Vec<PathBuf>,
    /// Paths adopted through the committed `.alef-ownership.toml` record because their
    /// format cannot carry a marker at all. Reported separately from [`Self::adopted`]
    /// because adopting these leaves the file itself byte-identical: the consent lives
    /// in a *different* file, which the operator has to commit for the adoption to mean
    /// anything anywhere else. ~keep
    pub recorded_unstampable: Vec<PathBuf>,
    /// Matches alef can make no statement about at all, in path order.
    ///
    /// Two shapes reach this list, and both are alef failing rather than the operator
    /// having a decision to make: bytes that are not valid UTF-8 on a path alef does not
    /// emit as binary, and a [`crate::cli::pipeline::is_base64_binary_output`] path whose
    /// generated content did not decode as base64. Skipped rather than adopted, and
    /// reported rather than dropped -- one such file used to abort the entire target: a
    /// single `gradle-wrapper.jar` inside `packages/**` ended the run with "stream did not
    /// contain valid UTF-8" before any of the hundreds of adoptable text files beside it
    /// were stamped.
    ///
    /// A jar is no longer one of them. It is classified on its decoded bytes and adopted
    /// through the ownership record like any other unstampable format -- see
    /// [`BinaryFacts`] for why refusing it was wrong. ~keep
    pub unreadable: Vec<PathBuf>,
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

/// Whether alef emits `file` only when its path is absent on disk — a create-once seed
/// subject to [`AdoptOptions::clobber_create_once_seeds`].
///
/// [`crate::core::backend::GeneratedFile::carries_alef_marker`] answers this correctly
/// for every ordinary generated path, and it is the whole answer for a real seed: alef
/// could have marked a `.zig` test stub and deliberately did not, so the missing marker
/// *is* the protection.
///
/// It is the wrong answer for an artifact whose format cannot hold a marker at all.
/// Such a file presents the identical signature — no marker, `generated_header: false`,
/// no ownership record until alef's first authorised write establishes one — while
/// meaning the exact opposite: there is no human-grown content to protect, and being
/// replaced wholesale is the intended behaviour. The snippet-coverage ledger was the
/// first such artifact and was answered here by its own file name, which meant this call
/// site and `write_scaffold_files_report`'s guard each carried a copy of the exception —
/// two places to keep in step, and nothing that generalises to the next unmarkable
/// artifact. [`crate::cli::cache::is_alef_derived_output`] is the one named property both
/// sides ask instead, and its doc carries the admission criteria a candidate has to
/// satisfy before it is added. ~keep
///
/// `pub(crate)` so [`crate::bin_cli::helpers::frozen::frozen_managed_paths`] can classify
/// a frozen path by the identical predicate this module gates
/// `--clobber-create-once-seeds` on, rather than re-deriving its own notion of "seed".
/// Before that sharing, `alef verify` reported every frozen create-once seed under the
/// same heading and the same "run `alef adopt <path>`" remedy as a genuinely adoptable
/// frozen file -- a remedy `alef adopt --write` then refused outright for every seed,
/// naming a flag verify never mentioned. Two components computing the same fact
/// independently is this codebase's most common defect shape; a single predicate both
/// sides call is the fix, not a second copy kept in step by hand. ~keep
pub(crate) fn is_create_once_seed(file: &crate::core::backend::GeneratedFile) -> bool {
    !file.carries_alef_marker() && !crate::cli::cache::is_alef_derived_output(&file.path)
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
            if crate::cli::pipeline::is_base64_binary_output(&file.path) {
                // Verbatim, because the writers decode `file.content` verbatim. Running the
                // text normalizer over base64 appends the trailing newline every text output
                // gets, and the decoder then rejects the whole payload -- so the artifact alef
                // would actually write becomes unrepresentable here, and every binary match
                // classifies as "alef cannot read this" rather than by its bytes. ~keep
                return ManagedOutput {
                    relative: file.path.clone(),
                    content: file.content.clone(),
                    create_once: is_create_once_seed(file),
                };
            }
            let normalized = crate::cli::pipeline::normalize_content(&full_path, &file.content);
            let content = if file.generated_header {
                crate::cli::pipeline::ensure_generated_header(&full_path, &normalized)
            } else {
                normalized
            };
            ManagedOutput {
                relative: file.path.clone(),
                content,
                create_once: is_create_once_seed(file),
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
///
/// `pub(crate)` so `bin_cli::helpers::collect_managed_surface` can ask the same
/// question adopt's own candidate selection asks -- "could this stage's output ever
/// satisfy one of these targets" -- from the identical predicate, rather than a second,
/// hand-maintained notion of what counts as a match. ~keep
pub(crate) fn matches_target(target: &str, relative: &Path) -> bool {
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
/// Three routes, tried in order:
///
/// 0. **Self-marking content whose body already converged**, driven by the *generated*
///    bytes rather than the path or a reconstructed header. A self-marking backend
///    (custom Swift/Kotlin/Dart/Gleam/Zig headers, and Markdown via route 2 below) bakes
///    its own marker text straight into `generated` -- wording route 1's generic
///    `hash::header` never produces, because these paths are `generated_header: false`
///    by design. Stamping `existing` with the generic header instead would then never
///    equal `generated` byte-for-byte even when the body is identical, so [`classify`]
///    would call a header-wording difference a body drift -- and the diff printed to
///    justify that verdict shows no changed body line at all, which is exactly the
///    signal an operator is told to read before consenting. When `generated` already
///    ends with `existing` verbatim, its leading bytes *are* the exact marker a
///    subsequent `alef generate` will (re)write for this self-marked path, so handing
///    `generated` itself back as the stamped bytes is both correct and format-agnostic --
///    no per-backend header table needed, and it changes no body byte because the body
///    is `existing` by construction of the check. Gated on `content_has_alef_marker`
///    too, so a coincidental byte-for-byte suffix match on an unmarked path can never
///    fire this route and skip stamping a marker entirely. ~keep
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
///    strictly better (it cannot be separated from the file it describes). Route 0
///    above already handles the common converged case for `.md` too (`generated` ends
///    with `existing` whenever the snippet body itself is unchanged); this route
///    remains for the drifted case, where route 0's suffix check cannot match. ~keep
fn stamp_for(full_path: &Path, existing: &str, generated: &str) -> Option<String> {
    if !existing.is_empty()
        && crate::core::hash::content_has_alef_marker(generated)
        && let Some(prefix) = generated.strip_suffix(existing)
        && crate::core::hash::is_provenance_only_prefix(prefix)
    {
        return Some(generated.to_owned());
    }
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
pub fn classify(
    full_path: &Path,
    relative: &Path,
    generated: &str,
    existing: &str,
    create_once: bool,
) -> AdoptCandidate {
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
        create_once,
        binary: None,
    }
}

/// Classify a [`crate::cli::pipeline::is_base64_binary_output`] match against the bytes
/// alef would decode onto it.
///
/// `stamped` is `None` by construction: a jar can carry no comment, so adoption goes
/// through the committed `.alef-ownership.toml` record — which is *already* the proof
/// route both writers consult for binary paths, not a new one invented here. [`apply`]'s
/// existing `None` arm therefore needs no binary-specific branch.
///
/// [`AdoptionState::AlreadyOwned`] is read off that same record rather than off the file,
/// mirroring the text rail exactly: a path alef has already proven it owns has no consent
/// left to give, whatever its current bytes.
///
/// Returns `None` when the generated content is not decodable base64 — an alef bug rather
/// than an operator decision, and one that must not abort the hundreds of adoptable files
/// under the same glob. The path is reported through [`AdoptReport::unreadable`] instead.
fn classify_binary(
    base_dir: &Path,
    full_path: &Path,
    output: &ManagedOutput,
    existing: &[u8],
) -> Option<AdoptCandidate> {
    let generated = match crate::cli::pipeline::decode_base64_binary(&output.relative, &output.content) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                path = %output.relative.display(),
                "cannot be adopted: alef's own generated content for this binary path did not decode: {error:#}"
            );
            return None;
        }
    };
    let state = if crate::cli::cache::is_scaffold_owned_path(base_dir, full_path) {
        AdoptionState::AlreadyOwned
    } else if existing == generated.as_slice() {
        AdoptionState::Converged
    } else {
        AdoptionState::Drifted
    };
    Some(AdoptCandidate {
        relative: output.relative.clone(),
        full_path: full_path.to_path_buf(),
        existing: String::new(),
        generated: String::new(),
        state,
        stamped: None,
        create_once: output.create_once,
        binary: Some(BinaryFacts::new(existing, &generated)),
    })
}

/// Render the complete line-by-line diff between what is on disk and what alef would
/// generate. Never truncated — see this module's header for why.
///
/// Called for drifted candidates only. A converged candidate has no divergence to
/// render, so its diff would be the whole file repeated as context lines.
pub fn render_diff(candidate: &AdoptCandidate) -> String {
    let spelled = candidate.relative.display();
    let mut body = format!("--- {spelled} (on disk)\n+++ {spelled} (alef generate output)\n");
    if let Some(facts) = &candidate.binary {
        // Deliberately shaped like the line diff above -- same header, same `-`/`+`
        // sides -- because it is answering the same question and lands in the same
        // reviewed output. Saying only "binary file differs" would name a fact the
        // operator already knows from the path and give them nothing to decide with. ~keep
        body.push_str("Binary output: no line diff exists. The bytes on each side:\n");
        body.push_str(&format!(
            "-{:>12} bytes  blake3:{}\n",
            facts.existing_len, facts.existing_digest
        ));
        body.push_str(&format!(
            "+{:>12} bytes  blake3:{}\n",
            facts.generated_len, facts.generated_digest
        ));
        return body;
    }
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

/// What [`collect_candidates`] found: the classifiable matches, plus the matches alef
/// could neither read as text nor decode as one of its own binary outputs.
struct CollectedCandidates {
    candidates: Vec<AdoptCandidate>,
    unreadable: Vec<PathBuf>,
}

/// Collect every managed output the target selects, refusing paths alef does not
/// generate and paths that do not exist yet.
fn collect_candidates(options: &AdoptOptions, managed: &[ManagedOutput]) -> Result<CollectedCandidates> {
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
    let mut unreadable: Vec<PathBuf> = Vec::new();
    for output in matched {
        let full_path = options.base_dir.join(&output.relative);
        if !full_path.exists() {
            // Nothing to consent to: the guard never engages on a path with no
            // pre-existing content, so an ordinary generate already writes this. ~keep
            continue;
        }
        let bytes =
            std::fs::read(&full_path).with_context(|| format!("failed to read existing {}", full_path.display()))?;
        if crate::cli::pipeline::is_base64_binary_output(&output.relative) {
            match classify_binary(&options.base_dir, &full_path, output, &bytes) {
                Some(candidate) => candidates.push(candidate),
                None => unreadable.push(output.relative.clone()),
            }
            continue;
        }
        let Ok(existing) = String::from_utf8(bytes) else {
            unreadable.push(output.relative.clone());
            continue;
        };
        candidates.push(classify(
            &full_path,
            &output.relative,
            &output.content,
            &existing,
            output.create_once,
        ));
    }

    if candidates.is_empty() && unreadable.is_empty() {
        bail!(
            "`{}` matches alef-managed output but nothing exists on disk yet -- \
             run `alef generate`, there is no ownership conflict to resolve",
            options.target
        );
    }
    Ok(CollectedCandidates { candidates, unreadable })
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
    let CollectedCandidates { candidates, unreadable } = collect_candidates(options, managed)?;
    let mut report = AdoptReport {
        preview: !options.write,
        unreadable,
        ..AdoptReport::default()
    };

    // Partitioned before anything is classified for the report, so an excluded seed
    // contributes no diff, no converged tally and no adopted entry -- it is not a thing
    // this run is deciding about, it is a thing this run refuses to decide about.
    //
    // `AlreadyOwned` seeds are deliberately not excluded: the file already carries a
    // marker, so there is no consent left to give and no content this command could put
    // at risk. Listing them as blocked would be a false alarm on the exact list whose
    // whole value is that every line on it is genuinely dangerous. ~keep
    let (adoptable, blocked): (Vec<&AdoptCandidate>, Vec<&AdoptCandidate>) = candidates.iter().partition(|candidate| {
        options.clobber_create_once_seeds || !candidate.create_once || candidate.state == AdoptionState::AlreadyOwned
    });
    for candidate in &blocked {
        report.skipped_create_once.push(candidate.relative.clone());
        tracing::warn!(
            path = %candidate.relative.display(),
            "create-once seed: alef emits this path only when absent, so adopting it consents to alef \
             replacing its contents with a placeholder seed on the next overwriting regen (an \
             `alef version` sync, `alef all --clobber-create-once-seeds`) -- a plain `alef generate` \
             skips it"
        );
    }

    for candidate in &adoptable {
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

    // Non-zero only when the exclusion emptied the run, so a mixed glob still does its
    // legitimate work instead of forcing the operator to re-type a narrower target (and
    // learn, from the failure, that the way to make adopt succeed is to pass the
    // dangerous flag). A `--write` that adopted nothing at all is a different matter: it
    // exits 0 today only because "nothing matched" is already a `bail!`, and staying
    // silent here would let a seeds-only glob look like a successful adoption. ~keep
    let has_work = adoptable.iter().any(|c| c.state != AdoptionState::AlreadyOwned);
    if !has_work && !report.skipped_create_once.is_empty() {
        bail!(
            "`{}` matches only create-once seeds, which alef emits solely when absent -- \
             adopting one consents to alef replacing its contents with a placeholder seed on the \
             next overwriting regen (an `alef version` sync, `alef all --clobber-create-once-seeds`), \
             so nothing was written. Pass --clobber-create-once-seeds to adopt them anyway.",
            options.target
        );
    }

    let mut to_record: Vec<PathBuf> = Vec::new();
    for candidate in adoptable.iter().filter(|c| c.state != AdoptionState::AlreadyOwned) {
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
