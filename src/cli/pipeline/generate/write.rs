use super::normalization::normalize_content;
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::hash;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, warn};

#[derive(Debug, Default)]
pub struct WriteReport {
    pub expected_paths: std::collections::HashSet<std::path::PathBuf>,
    pub changed_paths: std::collections::HashSet<std::path::PathBuf>,
    /// Paths the ownership guard declined to write.
    ///
    /// Recorded rather than only logged, because a refusal is otherwise invisible to every
    /// downstream signal: the guard `continue`s before the path reaches `expected_paths`, so
    /// orphan sweeps, freshness checks and the changed count all behave as though alef never
    /// intended to write the file. A permanently frozen file is then indistinguishable from
    /// one alef simply does not manage.
    ///
    /// Unlike an ordinary skip the condition never clears on its own, and the remedy —
    /// `alef adopt` — is a human action. A human cannot act on a number nobody reports, so
    /// this has to be visible rather than inferred from what did not change.
    ///
    /// A `BTreeSet` rather than a `Vec`: the same path can be refused by more than one guard
    /// site in a run, and the report is read by a person, so it must not repeat itself or
    /// reorder between runs. ~keep
    pub refused_paths: std::collections::BTreeSet<std::path::PathBuf>,
}

impl WriteReport {
    pub fn changed_count(&self) -> usize {
        self.changed_paths.len()
    }

    pub fn expected_count(&self) -> usize {
        self.expected_paths.len()
    }

    pub fn refused_count(&self) -> usize {
        self.refused_paths.len()
    }

    /// Fold another phase's refusals into this report.
    ///
    /// A run writes through several independent phases — bindings, service API, type stubs,
    /// public API, scaffolding — each returning its own report. The refusal summary is a
    /// run-level fact addressed to an operator, so reporting per phase understates it: the
    /// reader works the list they were shown and is left with the refusals from every other
    /// phase, unlisted and with no remaining signal that they exist. Only `refused_paths`
    /// merges; the changed and expected sets stay per-phase because their counts are reported
    /// per phase and summing them would double-count a path two phases both intended. ~keep
    pub fn absorb_refusals(&mut self, other: &WriteReport) {
        self.refused_paths.extend(other.refused_paths.iter().cloned());
    }
}

/// Surface every write the ownership guard declined, naming the remedy.
///
/// The guard is self-perpetuating by construction: it refuses because the file carries no
/// marker, and the marker can only arrive by writing the file. No later run breaks that
/// cycle, so a per-file `warn!` mid-run understates the situation — the condition is
/// permanent rather than transient, and only an operator can clear it. One consolidated
/// block naming the fix is the difference between a log line and an actionable report. ~keep
pub fn report_refused_writes(report: &WriteReport) {
    if report.refused_paths.is_empty() {
        return;
    }
    let mut paths: Vec<&std::path::PathBuf> = report.refused_paths.iter().collect();
    paths.sort();
    warn!(
        "{} file(s) were NOT written: each already exists, carries no alef provenance marker, and \
         alef has no durable record of owning it. This will not resolve on its own — the marker can \
         only be written by writing the file, which is exactly what the guard declines. Review the \
         diff for each and adopt the ones alef should own with `alef adopt <path>`. At migration \
         scale, `alef adopt <glob>` previews the whole set and `alef adopt <glob> --converged-only \
         --write` clears every file that already matches generated output, leaving the drifted \
         ones for you to read one at a time. If these are \
         formats that cannot carry a marker (package.json, *.jar) and this is a fresh clone or a CI \
         checkout, check whether .alef-ownership.toml was committed — that file is where their \
         ownership is recorded. Do NOT hand-add the marker line: a refusal can be protecting a \
         deliberate hand-edit, and stamping it blind re-enables exactly the clobbering the guard \
         exists to prevent.",
        paths.len()
    );
    for path in paths {
        warn!("  not written: {}", path.display());
    }
}

/// Every path in `files` that [`finalize_hashes`] must re-stamp this run.
///
/// This is deliberately a *disk-aware superset* of
/// [`GeneratedFile::carries_alef_marker`], and the difference between the two is a whole
/// bug class rather than an edge case. The two sides of the stamp contract answer the
/// ownership question from different evidence:
///
/// - the stamper asked the in-memory [`GeneratedFile`] — `generated_header`, or a marker
///   the emitter templated into `content`;
/// - `alef verify` ([`crate::bin_cli::helpers::collect_alef_hashes`]) asks the bytes **on
///   disk** — every file whose leading lines carry the marker is held to its stamp.
///
/// A create-once scaffold seed (`generated_header: false`, no marker in `content`:
/// `packages/go/go.mod`, `packages/zig/build.zig.zon`, the Swift `RustBridge` placeholder)
/// that an *earlier* alef wrote with a header sits on exactly that fault line. The
/// scaffold writer's `can_skip` never rewrites it, and it failed the stamper's in-memory
/// predicate, so no run re-stamped it — while verify, reading the marker off disk, held it
/// to a stamp frozen at whichever `inputs_hash` last wrote it. The first input change after
/// that reported the file stale permanently: regenerating could not clear it (there was
/// nothing to write), `alef adopt` refused it as already alef-owned, and the file was
/// content-correct the entire time. Deciding the stamp scope from the same evidence verify
/// uses is what makes that state unreachable, rather than a list of names to special-case.
///
/// Only the hash line is rewritten by the stamping pass this feeds; the file's body is
/// never touched, so a seed a human has grown past alef's placeholder keeps every byte it
/// has. ~keep
pub fn stampable_output_paths(
    files: &[GeneratedFile],
    base_dir: &Path,
) -> std::collections::HashSet<std::path::PathBuf> {
    files
        .iter()
        .filter_map(|file| {
            let full_path = base_dir.join(&file.path);
            // Short-circuits before touching the filesystem for the common case, so the
            // disk read only happens for the files the in-memory predicate rejects. ~keep
            (file.carries_alef_marker() || disk_carries_alef_marker(&full_path)).then_some(full_path)
        })
        .collect()
}

/// Whether the file already on disk at `path` declares itself alef-generated.
fn disk_carries_alef_marker(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|content| hash::content_has_alef_marker(&content))
}

pub fn managed_output_paths(files: &[GeneratedFile], base_dir: &Path) -> std::collections::HashSet<std::path::PathBuf> {
    files
        .iter()
        .filter(|file| file.carries_alef_marker())
        .map(|file| base_dir.join(&file.path))
        .collect()
}

pub fn managed_generated_files(files: &[GeneratedFile]) -> Vec<GeneratedFile> {
    files
        .iter()
        .filter(|file| file.carries_alef_marker())
        .cloned()
        .collect()
}

pub(crate) fn atomic_write(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("generated output path has no parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    if let Ok(metadata) = std::fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .with_context(|| format!("failed to preserve permissions for {}", path.display()))?;
    }
    std::io::Write::write_all(&mut temporary, content)
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

/// The **ownership** predicate: extensions where a missing `alef:hash:` marker is
/// treated as proof alef never authored the file.
///
/// Deliberately *narrower* than [`marker_header_syntax`], which is the **emit**
/// predicate. Both `write_files_report` and
/// [`super::scaffold::write_scaffold_files_report`] read `is_some()` here as
/// "absence of a marker is evidence of foreign authorship", and refuse to
/// overwrite on that basis. Adding an extension here therefore retroactively
/// freezes every already-existing file of that extension in every consumer repo
/// that does not carry a marker *yet*: the guard refuses the write, so the marker
/// can never land, so the guard refuses forever (the #77/#84 create-once trap).
/// Extensions graduate onto this list only after a release has been emitting a
/// marker for them long enough that consumer trees actually carry one; until then
/// they stay `None` and prove ownership through
/// [`crate::cli::cache::is_scaffold_owned_path`] as before.
///
/// `None` is load-bearing beyond formatting: a file alef cannot stamp never carries
/// an `alef:hash:` marker even when alef authored every byte of it (`.md` READMEs are
/// the widest instance — none of the generated per-language READMEs has ever had one).
/// So for those paths a missing marker is NOT evidence the file is foreign, and any
/// ownership check keyed on the marker must exempt them or it will freeze legitimate
/// regeneration forever. ~keep
///
/// Shared with version-sync's catch-all guard, which keys on exactly this
/// distinction — duplicating the extension table there would let the two drift and
/// silently change which files a rewrite is willing to touch. ~keep
pub(crate) fn marker_comment_style(path: &Path) -> Option<hash::CommentStyle> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("py" | "rb" | "r" | "ex" | "exs" | "toml" | "yaml" | "yml" | "sh") => Some(hash::CommentStyle::Hash),
        Some("h" | "hpp") => Some(hash::CommentStyle::Block),
        Some(
            "c" | "cc" | "cpp" | "cs" | "dart" | "gleam" | "go" | "java" | "js" | "kt" | "kts" | "php" | "rs" | "swift"
            | "ts" | "tsx" | "zig",
        ) => Some(hash::CommentStyle::DoubleSlash),
        _ => None,
    }
}

/// Every source of durable, committed evidence that alef owns `path`, for the formats
/// [`marker_comment_style`] answers `None` for -- the ownership fallback both write guards
/// and [`crate::bin_cli::helpers::find_missing_and_frozen_generated_files`]'s frozen-file
/// check must consult identically.
///
/// A single predicate on purpose. `write_files_report` and
/// `super::scaffold::write_scaffold_files_report` each used to inline their own copy of
/// this OR-chain, and the two had already drifted once -- the derived-output disjunct
/// (`is_alef_derived_output`) landed on this writer's copy only, and the snippet-ledger
/// disjuncts (`is_snippet_coverage_manifest_path`, `is_ledger_owned_snippet_path`) landed
/// on the scaffold writer's copy only. `alef verify`'s frozen-file report combines output
/// from every generation stage (`collect_managed_surface`) without knowing which writer
/// would eventually place a given path, so it needs the full union regardless -- and
/// unifying all three callers onto it closes the drift for good rather than adding a
/// fourth, independently-maintained copy. ~keep
pub(crate) fn is_owned_by_ownership_record(base_dir: &Path, path: &Path) -> bool {
    crate::cli::cache::is_scaffold_owned_path(base_dir, path)
        || crate::cli::cache::is_alef_derived_output(path)
        || crate::e2e::snippets::is_snippet_coverage_manifest_path(path)
        || crate::e2e::snippets::ownership::is_ledger_owned_snippet_path(base_dir, path)
}

/// How alef renders a provenance marker into a given file, or `None` when the
/// format genuinely has no comment syntax (`.json`) or alef must not write one
/// (lockfiles, which their own tool rewrites wholesale).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkerSyntax {
    /// One of [`hash::CommentStyle`]'s forms, rendered by [`hash::header`].
    Comment(hash::CommentStyle),
    /// `<!-- ... -->`, for XML-family formats. [`hash::CommentStyle`] has no
    /// variant for this, but every read-side function already understands the
    /// shape — [`hash::inject_hash_line`], [`hash::inject_stamp_line`],
    /// [`hash::extract_stamp`] and `hash::parse_generated_hash_line` all branch on
    /// a leading `<!--` — because `docs::render::with_html_header` has been
    /// emitting exactly this header for `.md` docs pages and READMEs all along.
    Html,
}

/// The **emit** predicate: which syntax [`ensure_generated_header`] stamps a marker
/// in. Covers strictly more paths than [`marker_comment_style`] — see that
/// function's doc for why the two must not be merged.
///
/// Widening this side is safe in a way widening the ownership side is not: a
/// header is only ever added on a write the guard has already authorised, so no
/// file can be frozen by it. It is also the *preferred* fix for the record
/// fallback (alef #80): a marker lives in the file it describes and cannot be
/// separated from it, whereas a separate record can be deleted, moved or
/// gitignored away from the artifact it covers. Every format that can hold a
/// marker should end up here; only the ones that genuinely cannot (`.json`,
/// `.jar`) fall back to `cache::OWNERSHIP_MANIFEST`, which is committed for the
/// same reason — so a fresh clone and a warm dev machine agree.
///
/// Per-format basis, verified against each format's own grammar rather than
/// assumed from the extension:
/// - `.cmake` — CMake `#` line comments (`cmake-language(7)`); no position
///   constraint. This is the escalated `crates/*-ffi/cmake/*-config.cmake` case.
/// - `.xml`, `.csproj` — XML comments. **Position-constrained**: XML 1.0 §2.8
///   requires the `<?xml ...?>` declaration to be the very first thing in the
///   document, so when one is present the marker goes on the line *after* it,
///   never at line 0. MSBuild `.csproj` is plain XML and usually omits the
///   declaration, in which case the marker leads.
/// - `Makefile` — `#` line comments. Matched on file *name*, since a makefile has
///   no extension.
/// - `go.mod` — `//` line comments (Go modules reference). Matched on file name,
///   not the `.mod` extension, which is shared with unrelated (and binary)
///   formats such as Fortran module files and tracker music.
/// - `.zon` — Zig Object Notation is read by the Zig tokenizer, so `//` line
///   comments apply, as in `.zig`.
/// - `.gemspec` — evaluated as Ruby, so `#` line comments apply.
/// - `Rakefile` — evaluated as Ruby, so `#` line comments apply. Matched on file
///   name; a Rakefile has no extension.
/// - `Makevars`, `Makevars.in`, `Makevars.win.in` — R's per-package make fragments,
///   read by make, so `#` line comments apply. Matched on file name: `Path::extension`
///   yields `in` for `Makevars.in`, which is far too generic to key on.
/// - `.clang-format` — YAML (clang-format's config grammar), so `#` line comments apply, as
///   in `.yaml`/`.yml`. Matched on file *name*: a dotfile with exactly one leading dot and no
///   further dot reports `Path::extension() == None` (the same rule that makes `.gitignore`
///   extensionless), so the `.yaml`/`.yml` arm of [`marker_comment_style`] never sees it.
///   Scaffolded `generated_header: true` (`scaffold/languages/poly.rs`) for every FFI target,
///   so it was silently unstamped the same way `Rakefile`/`Makevars*` were before those were
///   listed here, and for the same reason: nothing before this entry reported the mismatch.
///
/// `Rakefile` and `Makevars*` are emitted `generated_header: true` (`scaffold/languages/ruby.rs`,
/// `scaffold/languages/r.rs`), so before they were listed here `ensure_generated_header` was
/// called on them and silently returned the content unchanged — on the marker rail by intent,
/// off it in fact, with nothing reporting the discrepancy. ~keep
///
/// Deliberately excluded:
/// - `DESCRIPTION` (R packages) — also emitted `generated_header: true` and therefore
///   silently unstamped today, but left alone on purpose: it is Debian Control File
///   format, whose comment support is not the plain `#`-anywhere rule the other `#`
///   formats have, and this table's standard is a verified per-format grammar rather
///   than an assumption from adjacency. Stamping it on a guess risks corrupting a file
///   `R CMD build` parses strictly. Needs a DCF-grammar check before it graduates. ~keep
/// - `.json` — genuinely has no comment syntax in the spec. Unfixable; these keep
///   the `.alef/`-record fallback permanently.
/// - `.lock` — markability varies by lockfile (`Cargo.lock` is TOML and takes `#`;
///   `package-lock.json` takes nothing), but the distinction is moot: every
///   lockfile is written by its own package manager, which rewrites the file
///   wholesale and would drop an alef marker on the next resolve. alef does not
///   author them.
/// - `.md` — markable via HTML comments, but solved upstream instead: every producer of
///   generated Markdown routes its content through `docs::render::with_html_header`, which
///   embeds the identical marker before this function is ever reached. Adding it here would
///   be redundant at best and would newly stamp unrelated markdown at worst.
///
///   "Every producer" is a standing obligation on new `.md` emitters, not a property of the
///   extension, and it has already been violated once: `readme::template` and `docs::render`
///   honoured it, but `e2e::snippets::render_snippet_markdown` — a third `.md` producer added
///   later — did not, so fixture snippets were emitted with no marker from any side and the
///   write guard froze ~12k of them across two consumer repos. Fixed by routing that producer
///   through the same header, not by listing `.md` here: listing it would flip `.md` from
///   "unmarkable, proven by the ownership record" to "unmarkable-marker is proof of foreign
///   authorship" and retroactively freeze every already-existing unmarked `.md` in every
///   consumer repo, which is the create-once trap this table's sibling doc describes. `.md`
///   graduates only after a release has been marking snippets long enough that consumer trees
///   actually carry one. ~keep
pub(super) fn marker_header_syntax(path: &Path) -> Option<MarkerSyntax> {
    if let Some(style) = marker_comment_style(path) {
        return Some(MarkerSyntax::Comment(style));
    }
    match path.file_name().and_then(|name| name.to_str()) {
        Some("Makefile" | "GNUmakefile" | "makefile") => return Some(MarkerSyntax::Comment(hash::CommentStyle::Hash)),
        Some("go.mod") => return Some(MarkerSyntax::Comment(hash::CommentStyle::DoubleSlash)),
        Some("Rakefile" | "Makevars" | "Makevars.in" | "Makevars.win.in") => {
            return Some(MarkerSyntax::Comment(hash::CommentStyle::Hash));
        }
        Some(".clang-format") => return Some(MarkerSyntax::Comment(hash::CommentStyle::Hash)),
        _ => {}
    }
    // Case-folded, unlike the ownership predicate above. An extension's case is a spelling
    // convention, not a format: `.R` is THE conventional extension for an R script, and alef emits
    // `install.R`, `run_tests.R` and every `packages/r/R/*.R` with `generated_header: true` — all of
    // which `marker_comment_style`'s lowercase-only `"r"` arm skipped, so `ensure_generated_header`
    // returned them unstamped and the write guard then froze them for want of a marker nothing had
    // been emitting.
    //
    // Folding is confined to this emit predicate on purpose. Case-folding `marker_comment_style`
    // would reclassify `.R` from unmarkable to markable, and an unmarkable path proves ownership
    // through the committed record instead — so every already-committed `.R` in every consumer tree
    // would flip from "owned by the record" to "markable but unmarked", i.e. frozen, which is the
    // exact retroactive trap that function's doc warns against. Widening the emit side can only ever
    // add a header to a write the guard already authorised. ~keep
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("cmake" | "gemspec") => Some(MarkerSyntax::Comment(hash::CommentStyle::Hash)),
        Some("zon") => Some(MarkerSyntax::Comment(hash::CommentStyle::DoubleSlash)),
        Some("xml" | "csproj") => Some(MarkerSyntax::Html),
        Some(other) => marker_comment_style(Path::new("x").with_extension(other).as_path()).map(MarkerSyntax::Comment),
        None => None,
    }
}

/// Whether [`marker_header_syntax`] can stamp a file at `path` -- i.e. whether alef, having
/// decided to write this path, would put a provenance marker in its bytes.
///
/// Exists so `alef verify`'s ownership walk can derive its scan set from this emit table
/// instead of carrying a second, hand-maintained copy of it. That copy had already drifted:
/// `.clang-format` was added here (it is YAML, so `#` applies) while
/// `bin_cli::verify_scan::VERIFY_SCAN_FILENAMES` was not extended, and a dotfile with a single
/// leading dot reports no extension -- so every FFI target's stamped `.clang-format` was
/// written with a marker that nothing ever read back. A file the scan set omits is not merely
/// unverified, it is unverifiABLE: the walk filters on name and extension before it reads any
/// content, so the marker inside is unreachable no matter what it says. ~keep
pub(crate) fn is_markable_path(path: &Path) -> bool {
    marker_header_syntax(path).is_some()
}

/// Render the standard alef header as XML/HTML comments.
///
/// Derived from the `//` rendering rather than re-typing the body so the marker
/// text stays a single source of truth with [`hash::header`] — and so it stays
/// byte-identical to `docs::render::with_html_header`'s, which the `.md` side has
/// been emitting for as long as READMEs have proven ownership from content. ~keep
fn html_header() -> String {
    hash::header(hash::CommentStyle::DoubleSlash)
        .lines()
        .map(|line| format!("<!-- {} -->\n", line.strip_prefix("// ").unwrap_or(line)))
        .collect()
}

/// The literal header [`ensure_generated_header`] would prepend to a file at
/// `path`, purely from its path -- for a **generic** (`generated_header: true`)
/// emitter, whose in-memory `GeneratedFile::content` does not yet carry a
/// marker because this pass is what adds one at write time.
///
/// Exposed for `alef verify`'s frozen-file remedy message (a pre-existing file
/// alef would own but that carries no marker, so the write guard refuses it
/// forever -- see `bin_cli::helpers::find_missing_and_frozen_generated_files`),
/// so that message can quote the exact text a user would paste in rather than
/// a vague "add a marker" instruction. Returns `None` for the same paths
/// `ensure_generated_header` leaves untouched (`.json`, lockfiles): those
/// formats have no comment syntax to carry one.
///
/// Does **not** cover self-marking backends (custom Swift/Kotlin/Dart/Gleam/Zig
/// headers, `docs::render`'s HTML-commented `.md` pages) -- those already embed
/// their literal header text straight into `GeneratedFile::content`, so the
/// caller should read it from there instead of calling this. ~keep
pub(crate) fn provenance_header_for_path(path: &Path) -> Option<String> {
    match marker_header_syntax(path)? {
        MarkerSyntax::Comment(style) => Some(hash::header(style)),
        MarkerSyntax::Html => Some(html_header()),
    }
}

/// Split off a leading `<?xml ...?>` declaration, returning it and the remaining
/// body with the separating newline consumed.
///
/// Splits on the declaration's own `?>` terminator rather than on the first
/// newline, because a declaration may be the file's only line (no trailing
/// newline) or may wrap — and getting this wrong emits a comment *before* the
/// declaration, which is a hard XML parse error rather than a cosmetic slip. ~keep
fn split_xml_declaration(content: &str) -> Option<(&str, &str)> {
    let rest = content.strip_prefix("<?xml")?;
    let terminator = rest.find("?>")?;
    let split_at = "<?xml".len() + terminator + "?>".len();
    let (declaration, body) = content.split_at(split_at);
    Some((declaration, body.strip_prefix('\n').unwrap_or(body)))
}

pub(crate) fn ensure_generated_header(path: &Path, content: &str) -> String {
    if hash::content_has_alef_marker(content) {
        return content.to_owned();
    }

    let Some(syntax) = marker_header_syntax(path) else {
        return content.to_owned();
    };
    let header = match syntax {
        MarkerSyntax::Comment(style) => hash::header(style),
        MarkerSyntax::Html => html_header(),
    };
    if let Some((shebang, body)) = content.split_once('\n').filter(|(line, _)| line.starts_with("#!/")) {
        return format!("{shebang}\n{header}\n{body}");
    }
    if let Some((opening_tag, body)) = content.split_once('\n').filter(|(line, _)| line.trim() == "<?php") {
        return format!("{opening_tag}\n{header}\n{body}");
    }
    if let Some((declaration, body)) = split_xml_declaration(content) {
        return format!("{declaration}\n{header}\n{body}");
    }
    format!("{header}\n{content}")
}

/// Stamp `existing` with the provenance marker so a later run's ownership guard
/// recognises the file as alef's, returning `None` when the format has no marker
/// syntax at all and the caller must fall back to
/// [`crate::cli::cache::record_scaffold_owned_path`].
///
/// Content is preserved exactly: this only prepends the header
/// [`ensure_generated_header`] would have added, so adoption is a header-only edit
/// and the actual content convergence happens on the next ordinary `alef generate`,
/// through the guard, in full view of `git diff`.
///
/// **This is the only adoption route, and it is reachable only from `alef adopt`
/// ([`crate::cli::commands::adopt`]) — never from a write pass.** The create-once trap
/// that motivates adoption is real: a file whose type became stampable only after it
/// was already committed carries no marker, so the guard refuses the write, so the
/// marker never lands, so the guard refuses forever. `crates/*-ffi/Cargo.toml` in a
/// consumer repo is in exactly that state — `git log -S 'alef:hash'` returns nothing for
/// its entire history — and three landed fixes are frozen out of that repo by it.
///
/// An earlier revision escaped that trap automatically, with a `bootstrap_owned`
/// predicate that adopted any unmarked file whose bytes already equalled the run's
/// output minus the header. It was justified on the grounds that a hand-edited file
/// cannot reproduce the generator's bytes. That claim is false, and the counterexample
/// is the incident this guard exists for: a consumer's hand-written
/// `e2e/go/helpers_test.go` was byte-identical to alef's generated content, which is
/// exactly why the only visible damage was a stamped header. See
/// `scaffold_ownership_guard_tests` for the two regressions it caused.
///
/// The failure is not a fixable bug in that predicate. Ownership is a fact about
/// history — who authored these bytes — while a predicate sees only the bytes. "alef
/// wrote this under an older release" and "a human wrote this and it coincides" are the
/// same input, so no content test can separate them, however strict. The drifted case
/// is the same argument one step louder: adopting a drifted file is byte-for-byte
/// indistinguishable from clobbering a hand-edit, since both are "regenerated content
/// replaces different existing content". The only thing that separates them is a human
/// reading the diff, which is why `alef adopt` prints one and refuses to be folded into
/// `alef all`. Automating this would delete the guard while keeping the warning. ~keep
pub(crate) fn stamp_for_adoption(path: &Path, existing: &str) -> Option<String> {
    marker_header_syntax(path)?;
    Some(ensure_generated_header(path, existing))
}

/// Apply `0o755` permissions to a file whose content begins with a shebang line.
///
/// Called immediately after every `fs::write` in both [`write_files`] and
/// [`write_scaffold_files_with_overwrite`] so that generated shell scripts
/// (e.g. `download_ffi.sh`, `run_tests.sh`, `mvnw`) are executable on Unix
/// without a manual `chmod` step by the consumer.
///
/// On non-Unix platforms this is a no-op — POSIX permission bits do not exist.
#[cfg(unix)]
pub(crate) fn apply_shebang_chmod(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if content.starts_with("#!") {
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(path, perms).with_context(|| format!("failed to chmod 755 {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn apply_shebang_chmod(_path: &std::path::Path, _content: &str) -> anyhow::Result<()> {
    Ok(())
}

/// Write generated files to disk.
///
/// Rust files are formatted with `rustfmt` before writing so prek's `cargo fmt`
/// hook is a no-op on regenerated content. The embedded `alef:hash:<hex>`
/// value is a **per-file inputs+output** hash from [`hash::compute_file_hash`]:
/// `blake3("sources" || inputs_hash || "content" || file_content_without_hash_line)`,
/// where `inputs_hash` is [`hash::compute_inputs_hash`] (the generation-inputs
/// fingerprint, not the emitted file content).
///
/// Hashes are written in two passes by the caller:
/// 1. `write_files` writes content with the header but **no hash line** (the
///    header marker is left in place so [`finalize_hashes`] can find it later).
/// 2. After every formatter has run, the caller invokes [`finalize_hashes`]
///    to inject the per-file hash. This means the embedded hash always
///    reflects the actual on-disk byte content and `alef verify` is a
///    pure read+strip+rehash+compare with no regeneration.
pub fn write_files(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) -> anyhow::Result<usize> {
    Ok(write_files_report(files, base_dir)?.changed_count())
}

/// Writes binding/stub output for every configured language. Unlike
/// [`super::scaffold::write_scaffold_files_report`], this writer has no
/// create-only concept: virtually all of its traffic (FFI glue, service
/// dispatch, JNI shims, type stubs, ...) is 100% machine-owned and must be
/// regenerated on every run regardless of `generated_header` or content, so
/// the guard here never gates on either of those the way the scaffold
/// writer's `can_skip` does — it only ever asks "did alef write the content
/// that's already here," exactly like [`super::scaffold::write_scaffold_files_report`]'s
/// markable/unmarkable split:
///
/// - **Markable** ([`marker_comment_style`] is `Some`): the existing content
///   must already carry the `alef:hash:` marker.
/// - **Unmarkable** (`.pyi` type stubs, `.cmake` config, ...): proven instead
///   by [`crate::cli::cache::is_scaffold_owned_path`], the same `base_dir`-scoped
///   committed record (`.alef-ownership.toml`) `write_scaffold_files_report`
///   populates and consults — no crate name needed for either writer, since the
///   record is keyed on the full output path.
///
/// This was previously left unguarded for unmarkable extensions specifically
/// because no incident had been observed for this writer's output; that
/// premise no longer holds — cross-repo review surfaced this writer emitting
/// unmarkable, structurally-uncommentable output (`crates/*-ffi/cmake/*-config.cmake`)
/// alongside markable-but-plausibly-hand-touched FFI headers, and there is no
/// way to tell an alef-owned `.cmake` file from a foreign one by path or
/// extension alone — precisely the "known-generated-but-unstampable" gap this
/// route closes. The guard only ever engages when content would actually
/// change, so `frb_generated.rs`-style output that legitimately differs on
/// every run (new API surface, a bumped dependency) keeps regenerating
/// exactly as before, provided it already carries the marker (markable) or a
/// committed record (unmarkable) from the run that first wrote it. ~keep
pub fn write_files_report(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) -> anyhow::Result<WriteReport> {
    let mut prepared = std::collections::BTreeMap::<std::path::PathBuf, (Vec<u8>, bool)>::new();
    for file in files.iter().flat_map(|(_, lang_files)| lang_files.iter()) {
        let full_path = base_dir.join(&file.path);
        let (content, is_text) = if super::binary::is_base64_binary_output(&full_path) {
            (super::binary::decode_base64_binary(&full_path, &file.content)?, false)
        } else {
            let normalized = normalize_content(&full_path, &file.content);
            let normalized = if file.generated_header {
                ensure_generated_header(&full_path, &normalized)
            } else {
                if hash::content_has_alef_marker(&normalized) {
                    // The emitter opted out of the prepended header but templated a
                    // marker into the body anyway. Stamping follows the marker, so
                    // this is no longer harmful — surface it so the mismatch does not
                    // become invisible convention. ~keep
                    debug!(
                        "  {}: emitted with generated_header = false but body carries an alef marker",
                        full_path.display()
                    );
                }
                normalized
            };
            (normalized.into_bytes(), true)
        };
        if let Some((existing, _)) = prepared.get(&full_path) {
            anyhow::ensure!(
                existing == &content,
                "multiple generators emitted different content for {}",
                full_path.display()
            );
            continue;
        }
        prepared.insert(full_path, (content, is_text));
    }
    let dirs: std::collections::BTreeSet<_> = prepared
        .keys()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect();
    for dir in &dirs {
        std::fs::create_dir_all(dir).with_context(|| format!("failed to create directory {}", dir.display()))?;
    }

    let changed_paths = std::sync::Mutex::new(std::collections::HashSet::new());
    let refused_paths = std::sync::Mutex::new(std::collections::BTreeSet::new());
    let refuse = |path: &Path| {
        refused_paths
            .lock()
            .expect("refused-path mutex poisoned")
            .insert(path.to_path_buf());
    };
    prepared
        .par_iter()
        .try_for_each(|(full_path, (content, is_text))| -> anyhow::Result<()> {
            if *is_text {
                let normalized = std::str::from_utf8(content).context("prepared generated text was not UTF-8")?;
                let is_markable = marker_comment_style(full_path).is_some();
                if full_path.exists() {
                    let Ok(existing) = std::fs::read_to_string(full_path) else {
                        warn!(
                            "refusing to write {}: pre-existing file could not be read as text -- \
                             leaving it untouched",
                            full_path.display()
                        );
                        refuse(full_path);
                        return Ok(());
                    };
                    let existing_body = crate::core::hash::strip_hash_line(&existing);
                    let normalized_body = crate::core::hash::strip_hash_line(normalized);
                    if existing_body == normalized_body {
                        apply_shebang_chmod(full_path, normalized)?;
                        debug!("  unchanged: {}", full_path.display());
                        // Deliberately records nothing. Reaching here proves only that the
                        // bytes coincide with this run's output, which is not evidence of
                        // authorship — a consumer's hand-written `e2e/go/helpers_test.go`
                        // was byte-identical to alef's, and minting a claim from that is the
                        // `bootstrap_owned` predicate `stamp_for_adoption`'s doc removed,
                        // relocated into the record. Now that the record is committed
                        // (`cache::OWNERSHIP_MANIFEST`) the claim would also be permanent and
                        // shared, so a coincidence on one developer's disk would freeze into
                        // ownership for everyone. A file alef genuinely wrote was recorded by
                        // the authorised-write branch below on the run that created it. ~keep
                        return Ok(());
                    }
                    // Checked unconditionally, not only for markable extensions: content
                    // can self-mark on any extension (see `scaffold.rs`'s guard doc for the
                    // docs-page HTML-comment header that is exactly this case). The local
                    // ownership record is the fallback only for extensions that truly
                    // cannot carry a marker in any form.
                    let has_marker = hash::content_has_alef_marker(&existing);
                    // Delegates the unmarkable fallback to `is_owned_by_ownership_record` rather
                    // than inlining its own OR-chain: this guard and `scaffold.rs`'s guard used to
                    // carry two independently hand-maintained copies of "which committed record
                    // proves ownership", and they had already drifted once -- see that function's
                    // doc for the incident. One shared predicate is now the only place this list
                    // can grow. ~keep
                    let owned = has_marker || (!is_markable && is_owned_by_ownership_record(base_dir, full_path));
                    if !owned {
                        // Distinguishes "nothing here even tried" from "something here tried and
                        // got the spelling wrong" -- see `hash::near_miss_marker`'s doc. Kept
                        // deliberately identical to `scaffold.rs`'s guard, same as `owned` above. ~keep
                        match hash::near_miss_marker(&existing) {
                            Some(near_miss) => warn!(
                                "refusing to write {}: pre-existing file carries no alef marker and \
                                 alef has no durable record of ever owning it -- its leading lines \
                                 contain something close to a marker ({near_miss:?}) that alef does \
                                 not recognize; alef accepts \"generated by alef\" case-insensitively \
                                 -- leaving it untouched",
                                full_path.display()
                            ),
                            None => warn!(
                                "refusing to write {}: pre-existing file carries no alef marker and \
                                 alef has no durable record of ever owning it -- leaving it untouched",
                                full_path.display()
                            ),
                        }
                        refuse(full_path);
                        return Ok(());
                    }
                }
                atomic_write(full_path, content)?;
                apply_shebang_chmod(full_path, normalized)?;
                if !is_markable {
                    crate::cli::cache::record_scaffold_owned_path(base_dir, full_path)?;
                }
            } else {
                if full_path.exists() {
                    let existing_binary = std::fs::read(full_path).ok();
                    if existing_binary.as_deref() == Some(content.as_slice()) {
                        debug!("  unchanged: {}", full_path.display());
                        // Records nothing, for the same reason as the text branch above: a
                        // pre-existing binary that happens to match is not proof alef put it
                        // there, and a binary target has no marker route to correct the
                        // mistake later. ~keep
                        return Ok(());
                    }
                    if existing_binary.is_some() && !crate::cli::cache::is_scaffold_owned_path(base_dir, full_path) {
                        warn!(
                            "refusing to write {}: pre-existing file has no durable record of \
                             alef ownership -- leaving it untouched",
                            full_path.display()
                        );
                        refuse(full_path);
                        return Ok(());
                    }
                }
                atomic_write(full_path, content)?;
                crate::cli::cache::record_scaffold_owned_path(base_dir, full_path)?;
            }
            changed_paths
                .lock()
                .expect("changed-path mutex poisoned")
                .insert(full_path.clone());
            debug!("  wrote: {}", full_path.display());
            Ok(())
        })?;

    Ok(WriteReport {
        expected_paths: prepared.into_keys().collect(),
        changed_paths: changed_paths.into_inner().expect("changed-path mutex poisoned"),
        refused_paths: refused_paths.into_inner().expect("refused-path mutex poisoned"),
    })
}

/// Inject the per-file `alef:hash:` line into every alef-headered file in
/// `paths`. Run *after* every formatter (`format_generated`, `fmt_post_generate`).
///
/// The embedded hash covers the generation inputs and the final formatted file
/// body. Running this after all formatters makes manual output edits detectable
/// without treating Alef's own formatting pass as drift.
///
/// Files that don't carry the alef header marker (scaffold-once Cargo.toml,
/// composer.json, package.json, lockfiles) are skipped — alef has
/// no claim on them. The Ruby gemspec and `.rubocop.yml` are NOT in this category — both carry
/// the alef header (`generated_header: true`) and are alef-owned, overwritten on every `alef build`.
pub fn finalize_hashes(
    paths: &std::collections::HashSet<std::path::PathBuf>,
    sources_hash: &str,
    alef_toml_bytes: &[u8],
) -> anyhow::Result<usize> {
    let inputs_hash = hash::compute_inputs_hash(sources_hash, alef_toml_bytes);

    let updated: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    paths.par_iter().try_for_each(|path| -> anyhow::Result<()> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };
        if !hash::content_has_alef_marker(&content) {
            return Ok(());
        }

        let stripped = hash::strip_hash_line(&content);
        let file_hash = hash::compute_file_hash(&inputs_hash, &stripped);
        let final_content = hash::inject_hash_line(&stripped, &file_hash);

        if final_content == content {
            return Ok(());
        }

        atomic_write(path, final_content.as_bytes())?;
        apply_shebang_chmod(path, &final_content)?;
        updated.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    })?;
    Ok(updated.into_inner())
}

/// Like [`finalize_hashes`], but self-healing: before stamping, unions `paths`
/// with every alef-headered file already on disk under `roots` (via
/// [`super::orphans::collect_alef_headered_paths`]).
///
/// `finalize_hashes` only re-stamps the paths it is handed, and callers build
/// that set from **this run's** in-memory generated-file lists. A language
/// whose generation was skipped because its content hash matched the
/// per-language cache (`generation::generate`) contributes no files to that
/// list, so any output it owns never reaches `finalize_hashes` even if that
/// output is missing its `alef:hash:` line — e.g. because it was written by a
/// version of alef that stripped the hash on write and never got a chance to
/// finalize it, or because a previous run was interrupted between the two
/// passes. Once a file like that fails to appear in an explicit path set once,
/// pure path-tracking can never recover it: the same cache hit drops it again
/// on every subsequent run.
///
/// Scanning `roots` (the languages' own output directories -- see
/// [`super::orphans::generate_sweep_roots`] -- never the whole repository)
/// closes that gap by going to the filesystem instead of trusting in-memory
/// bookkeeping: every alef-headered file that physically exists under `roots`
/// gets its hash re-derived from its current on-disk content, regardless of
/// whether this run's generation touched it. Because the per-file stamping in
/// `finalize_hashes` is itself idempotent (it always recomputes from current
/// content and only writes when the result differs), sweeping the same file
/// twice -- once via explicit tracking, once via the directory scan -- is
/// harmless; `paths` is a `HashSet`; duplicates collapse before any file is
/// touched.
///
/// The same "regardless of whether this run's generation touched it" breadth
/// that makes the cache-hit self-heal work has a cost the self-heal framing
/// doesn't mention: `collect_alef_headered_paths` cannot tell a still-valid
/// skipped-language file from a file the generator has permanently stopped
/// emitting (a type folded into a capsule, a manifest whose emit condition
/// changed). Both carry the marker; both get re-derived from their own
/// on-disk content and re-stamped here. For the orphan, that is not
/// "harmless" the way the duplicate-sweep case is -- it is the step that
/// launders a stale file into one `alef verify` reports as current, with no
/// trace that it happened. `paths` (this run's explicit, generation-sourced
/// keep set) is the only signal this function has for which route a given
/// path took, so every path added purely by the directory scan is logged
/// below; it is not proof of staleness (a legitimately skipped language lands
/// here too), but it is the only place in this call chain the two cases are
/// still distinguishable at all, so it is surfaced rather than silently
/// re-stamped alongside the paths this run actually generated. ~keep
pub fn finalize_hashes_sweeping(
    paths: &std::collections::HashSet<std::path::PathBuf>,
    roots: &[std::path::PathBuf],
    sources_hash: &str,
    alef_toml_bytes: &[u8],
) -> anyhow::Result<usize> {
    let mut swept = paths.clone();
    for root in roots {
        swept.extend(super::orphans::collect_alef_headered_paths(root));
    }
    log_disk_scan_only_restamps(paths, &swept);
    finalize_hashes(&swept, sources_hash, alef_toml_bytes)
}

/// Name every path in `swept` that reached [`finalize_hashes_sweeping`] only
/// through the `collect_alef_headered_paths` directory scan, not through
/// `paths` (this run's explicit, generation-sourced keep set). See that
/// function's doc for why this is the one place the two routes are still
/// distinguishable, and why it is not by itself proof any given path is an
/// orphan. ~keep
fn log_disk_scan_only_restamps(
    paths: &std::collections::HashSet<std::path::PathBuf>,
    swept: &std::collections::HashSet<std::path::PathBuf>,
) {
    let mut disk_scan_only: Vec<&std::path::PathBuf> = swept.difference(paths).collect();
    if disk_scan_only.is_empty() {
        return;
    }
    disk_scan_only.sort();
    debug!(
        "{} alef-marked file(s) under this sweep's roots were re-stamped from their own on-disk \
         content without appearing in this run's explicit generation output -- expected for a \
         language skipped by the per-language cache, but also how a file the generator has \
         stopped emitting entirely (a dropped type, a manifest whose emit condition changed) gets \
         its stale `alef:hash:` line replaced with one that matches, so `alef verify` reports it \
         current. If a path below is not owned by a currently cache-skipped language, it is \
         orphaned rather than merely unchanged -- cross-check against sweep_manifest_orphans's \
         input for this run.",
        disk_scan_only.len()
    );
    for path in disk_scan_only {
        debug!("  re-stamped via disk scan only: {}", path.display());
    }
}

mod tree_stamp;
pub use tree_stamp::finalize_hashes_after_tree_format;

#[cfg(test)]
mod marker_syntax_tests;
#[cfg(test)]
mod stamp_scope_tests;
#[cfg(test)]
mod tree_format_stamp_tests;
