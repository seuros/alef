use super::normalization::normalize_content;
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::hash;
use anyhow::Context as _;
use base64::Engine;
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
         diff for each and adopt the ones alef should own with `alef adopt <path>`. If these are \
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
/// - `.md` — markable via HTML comments, but already solved upstream:
///   `readme::template` and `docs::render` both route content through
///   `docs::render::with_html_header`, which embeds the identical marker before
///   this function is ever reached. Adding it here would be redundant at best and
///   would newly stamp unrelated markdown at worst. ~keep
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
        _ => {}
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("cmake" | "gemspec") => Some(MarkerSyntax::Comment(hash::CommentStyle::Hash)),
        Some("zon") => Some(MarkerSyntax::Comment(hash::CommentStyle::DoubleSlash)),
        Some("xml" | "csproj") => Some(MarkerSyntax::Html),
        _ => None,
    }
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
/// marker never lands, so the guard refuses forever. `crates/*-ffi/Cargo.toml` in
/// crawlberg is in exactly that state — `git log -S 'alef:hash'` returns nothing for
/// its entire history — and three landed fixes are frozen out of that repo by it.
///
/// An earlier revision escaped that trap automatically, with a `bootstrap_owned`
/// predicate that adopted any unmarked file whose bytes already equalled the run's
/// output minus the header. It was justified on the grounds that a hand-edited file
/// cannot reproduce the generator's bytes. That claim is false, and the counterexample
/// is the incident this guard exists for: crawlberg's hand-written
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
        let (content, is_text) = if full_path.extension().is_some_and(|extension| extension == "jar") {
            (
                base64::engine::general_purpose::STANDARD
                    .decode(&file.content)
                    .with_context(|| format!("failed to decode base64 for {}", full_path.display()))?,
                false,
            )
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
                        // authorship — crawlberg's hand-written `e2e/go/helpers_test.go` was
                        // byte-identical to alef's, and minting a claim from that is the
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
                    let owned =
                        has_marker || (!is_markable && crate::cli::cache::is_scaffold_owned_path(base_dir, full_path));
                    if !owned {
                        warn!(
                            "refusing to write {}: pre-existing file carries no alef marker and \
                             alef has no durable record of ever owning it -- leaving it untouched",
                            full_path.display()
                        );
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
/// composer.json, gemspec, package.json, lockfiles) are skipped — alef has
/// no claim on them.
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
    finalize_hashes(&swept, sources_hash, alef_toml_bytes)
}

#[cfg(test)]
mod marker_syntax_tests {
    use super::{
        MarkerSyntax, ensure_generated_header, marker_comment_style, marker_header_syntax, provenance_header_for_path,
    };
    use crate::core::hash;
    use std::path::Path;

    const HASH: &str = "0ce4d753fdb4854e44358639dcbaebee3449a4afa142dbc4f0a72aa72c214648";

    const HASH_HEADER: &str = "# This file is auto-generated by alef — DO NOT EDIT.\n\
# To regenerate: alef generate\n\
# To verify freshness: alef verify\n";
    const SLASH_HEADER: &str = "// This file is auto-generated by alef — DO NOT EDIT.\n\
// To regenerate: alef generate\n\
// To verify freshness: alef verify\n";
    const HTML_HEADER: &str = "<!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\
<!-- To regenerate: alef generate -->\n\
<!-- To verify freshness: alef verify -->\n";

    /// Emit a header, then run the exact stamping pass `finalize_hashes` runs, and
    /// return both stages so a test can assert bytes at each.
    fn emit_and_stamp(path: &str, body: &str) -> (String, String) {
        let stamped = ensure_generated_header(Path::new(path), body);
        let hashed = hash::inject_hash_line(&stamped, HASH);
        (stamped, hashed)
    }

    /// The full read-side contract every newly markable type must satisfy: the
    /// marker is findable, the hash injected next to it round-trips out again, and
    /// stripping the hash line reproduces the pre-stamp bytes exactly.
    fn assert_read_side_agrees(stamped: &str, hashed: &str) {
        assert!(
            hash::content_has_alef_marker(stamped),
            "content_has_alef_marker must find the emitted marker in:\n{stamped}"
        );
        assert_eq!(
            hash::extract_hash(hashed),
            Some(HASH.to_owned()),
            "extract_hash must recover the injected hash from:\n{hashed}"
        );
        assert_eq!(
            hash::strip_hash_line(hashed),
            stamped,
            "strip_hash_line must reproduce the pre-stamp bytes exactly"
        );
    }

    #[test]
    fn should_stamp_cmake_config_with_hash_comment() {
        let (stamped, hashed) = emit_and_stamp("crates/foo-ffi/cmake/foo-ffi-config.cmake", "if(TARGET foo::foo)\n");
        assert_eq!(stamped, format!("{HASH_HEADER}\nif(TARGET foo::foo)\n"));
        assert_eq!(
            hashed,
            format!(
                "# This file is auto-generated by alef — DO NOT EDIT.\n\
# alef:hash:{HASH}\n\
# To regenerate: alef generate\n\
# To verify freshness: alef verify\n\
\n\
if(TARGET foo::foo)\n"
            )
        );
        assert_read_side_agrees(&stamped, &hashed);
    }

    /// The load-bearing position case: XML 1.0 §2.8 forbids anything, comments
    /// included, before the `<?xml ...?>` declaration, so the marker lands on line
    /// 1 rather than line 0 — and every read-side function has to tolerate that.
    #[test]
    fn should_stamp_xml_after_the_declaration_never_before_it() {
        let (stamped, hashed) = emit_and_stamp(
            "test_apps/php/phpunit.xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<phpunit/>\n",
        );
        assert_eq!(
            stamped,
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{HTML_HEADER}\n<phpunit/>\n")
        );
        assert_eq!(
            stamped.lines().next(),
            Some("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
            "the XML declaration must remain the very first bytes of the document"
        );
        assert_eq!(
            stamped.lines().nth(1),
            Some("<!-- This file is auto-generated by alef — DO NOT EDIT. -->"),
            "the marker belongs on line 1, immediately after the declaration"
        );
        assert_eq!(
            hashed,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\
<!-- alef:hash:{HASH} -->\n\
<!-- To regenerate: alef generate -->\n\
<!-- To verify freshness: alef verify -->\n\
\n\
<phpunit/>\n"
            )
        );
        assert_read_side_agrees(&stamped, &hashed);
    }

    /// A declaration that is the whole first line without a trailing newline still
    /// must not get a comment pushed in front of it.
    #[test]
    fn should_stamp_xml_whose_declaration_has_no_trailing_newline() {
        let (stamped, hashed) = emit_and_stamp("packages/java/pom.xml", "<?xml version=\"1.0\"?><project/>\n");
        assert_eq!(stamped, format!("<?xml version=\"1.0\"?>\n{HTML_HEADER}\n<project/>\n"));
        assert_read_side_agrees(&stamped, &hashed);
    }

    #[test]
    fn should_stamp_csproj_without_declaration_at_line_zero() {
        let (stamped, hashed) = emit_and_stamp(
            "e2e/csharp/Foo.E2eTests.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk\">\n</Project>\n",
        );
        assert_eq!(
            stamped,
            format!("{HTML_HEADER}\n<Project Sdk=\"Microsoft.NET.Sdk\">\n</Project>\n")
        );
        assert_read_side_agrees(&stamped, &hashed);
    }

    #[test]
    fn should_stamp_makefile_with_hash_comment() {
        let (stamped, hashed) = emit_and_stamp("e2e/c/Makefile", "all:\n\t$(CC) main.c\n");
        assert_eq!(stamped, format!("{HASH_HEADER}\nall:\n\t$(CC) main.c\n"));
        assert_read_side_agrees(&stamped, &hashed);
    }

    #[test]
    fn should_stamp_gemspec_with_ruby_hash_comment() {
        let (stamped, hashed) = emit_and_stamp("packages/ruby/foo.gemspec", "Gem::Specification.new do |s|\nend\n");
        assert_eq!(stamped, format!("{HASH_HEADER}\nGem::Specification.new do |s|\nend\n"));
        assert_read_side_agrees(&stamped, &hashed);
    }

    #[test]
    fn should_stamp_go_mod_with_double_slash_comment() {
        let (stamped, hashed) = emit_and_stamp("e2e/go/go.mod", "module example.com/e2e\n\ngo 1.24\n");
        assert_eq!(stamped, format!("{SLASH_HEADER}\nmodule example.com/e2e\n\ngo 1.24\n"));
        assert_read_side_agrees(&stamped, &hashed);
    }

    #[test]
    fn should_stamp_zon_with_zig_double_slash_comment() {
        let (stamped, hashed) = emit_and_stamp("packages/zig/build.zig.zon", ".{\n    .name = \"foo\",\n}\n");
        assert_eq!(stamped, format!("{SLASH_HEADER}\n.{{\n    .name = \"foo\",\n}}\n"));
        assert_read_side_agrees(&stamped, &hashed);
    }

    #[test]
    fn should_leave_json_untouched_because_it_has_no_comment_syntax() {
        let body = "{\n  \"name\": \"foo\"\n}\n";
        assert_eq!(
            ensure_generated_header(Path::new("packages/node/package.json"), body),
            body
        );
        assert_eq!(marker_header_syntax(Path::new("packages/node/package.json")), None);
    }

    /// Lockfiles are rewritten wholesale by their own package manager, which would
    /// drop an alef marker on the next resolve, so alef never stamps one.
    #[test]
    fn should_leave_lockfiles_untouched() {
        let body = "# This file is automatically @generated by Cargo.\nversion = 4\n";
        assert_eq!(ensure_generated_header(Path::new("Cargo.lock"), body), body);
        assert_eq!(marker_header_syntax(Path::new("e2e/php/composer.lock")), None);
    }

    /// `alef verify`'s frozen-file remedy relies on this returning the exact
    /// bytes `ensure_generated_header` would prepend, not a paraphrase --
    /// `should_preserve_existing_markable_extension_behaviour` already pins the
    /// comment-style rendering itself, so this only needs to confirm the two
    /// stay wired together for a comment-style and an HTML-style path each.
    #[test]
    fn provenance_header_for_path_matches_what_ensure_generated_header_would_prepend() {
        assert_eq!(
            provenance_header_for_path(Path::new("src/lib.rs")),
            Some(SLASH_HEADER.to_owned())
        );
        assert_eq!(
            provenance_header_for_path(Path::new("packages/java/pom.xml")),
            Some(HTML_HEADER.to_owned())
        );
    }

    /// Mirrors `should_leave_json_untouched_because_it_has_no_comment_syntax`:
    /// a format with no comment syntax has no marker line to hand back either.
    #[test]
    fn provenance_header_for_path_returns_none_for_an_unmarkable_extension() {
        assert_eq!(
            provenance_header_for_path(Path::new("packages/node/package.json")),
            None
        );
    }

    /// The emit table must stay strictly wider than the ownership table: every
    /// newly markable type has to keep proving ownership through the `.alef/`
    /// record, or every such file already on disk without a marker is frozen
    /// forever (the guard refuses the write, so the marker never lands).
    #[test]
    fn should_not_promote_newly_emitted_types_onto_the_ownership_table() {
        for path in [
            "crates/foo-ffi/cmake/foo-ffi-config.cmake",
            "test_apps/php/phpunit.xml",
            "e2e/csharp/Foo.E2eTests.csproj",
            "e2e/c/Makefile",
            "packages/ruby/foo.gemspec",
            "e2e/go/go.mod",
            "packages/zig/build.zig.zon",
        ] {
            assert!(
                marker_header_syntax(Path::new(path)).is_some(),
                "{path} must be stamped with a marker"
            );
            assert_eq!(
                marker_comment_style(Path::new(path)),
                None,
                "{path} must stay off the ownership table until markers have propagated \
                 to consumer repos, otherwise existing unmarked copies freeze permanently"
            );
        }
    }

    /// Existing markable extensions must keep their exact previous behaviour: the
    /// emit table delegates to the ownership table first, so nothing about `.rs`,
    /// `.py`, `.h` or the `<?php` / shebang prefix handling may shift.
    #[test]
    fn should_preserve_existing_markable_extension_behaviour() {
        assert_eq!(
            marker_header_syntax(Path::new("src/lib.rs")),
            Some(MarkerSyntax::Comment(hash::CommentStyle::DoubleSlash))
        );
        assert_eq!(
            marker_header_syntax(Path::new("foo.h")),
            Some(MarkerSyntax::Comment(hash::CommentStyle::Block))
        );
        let (stamped, hashed) = emit_and_stamp("scripts/run.sh", "#!/usr/bin/env bash\nset -e\n");
        assert_eq!(stamped, format!("#!/usr/bin/env bash\n{HASH_HEADER}\nset -e\n"));
        assert_read_side_agrees(&stamped, &hashed);

        let (php_stamped, php_hashed) = emit_and_stamp("src/Foo.php", "<?php\nclass Foo {}\n");
        assert_eq!(php_stamped, format!("<?php\n{SLASH_HEADER}\nclass Foo {{}}\n"));
        assert_read_side_agrees(&php_stamped, &php_hashed);
    }

    /// Content that already self-marks (README/docs pages via
    /// `docs::render::with_html_header`) must not gain a second header now that
    /// `.md`-style HTML markers are also emittable from this side.
    #[test]
    fn should_not_double_stamp_content_that_already_carries_a_marker() {
        let already = format!("{HTML_HEADER}\n<project/>\n");
        assert_eq!(
            ensure_generated_header(Path::new("packages/java/pom.xml"), &already),
            already
        );
    }

    /// The generalized stamp channel has to survive the line-1 marker position too,
    /// since `extract_stamp` scans for the marker before it starts matching keys.
    #[test]
    fn should_round_trip_a_generic_stamp_through_an_xml_declaration_prologue() {
        let stamped = ensure_generated_header(
            Path::new("test_apps/php/phpunit.xml"),
            "<?xml version=\"1.0\"?>\n<phpunit/>\n",
        );
        let with_stamp = hash::inject_stamp_line(&stamped, hash::HANDLE_ABI_STAMP_KEY, "2");
        assert_eq!(
            hash::extract_stamp(&with_stamp, hash::HANDLE_ABI_STAMP_KEY),
            Some("2".to_owned())
        );
    }
}
