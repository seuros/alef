use std::fs;
use std::path::{Path, PathBuf};

const CACHE_DIR: &str = ".alef";
const PER_FILE_CACHE_NAME: &str = "sources_hash.cache";

/// Read the raw bytes of the alef config file for use in [`crate::core::hash::compute_inputs_hash`].
///
/// Returns an empty `Vec` when the file is absent or unreadable — callers
/// treat missing bytes as "empty config", which still produces a stable hash
/// when combined with `sources_hash`.
pub fn read_alef_toml_bytes(config_path: &Path) -> Vec<u8> {
    fs::read(config_path).unwrap_or_default()
}

/// Compute the per-run sources hash that drives both the IR cache and the
/// embedded `alef:hash:` value. Pure function of the rust source files
/// (paths + content); independent of `alef.toml` and the alef CLI version, so
/// that `alef verify` is idempotent across alef upgrades.
///
/// Warm-run optimisation: stat every source and check `(mtime_nanos, size)`
/// against an on-disk memo (`.alef/sources_hash.cache`). When **every** file's
/// stat is unchanged we return the cached aggregate hash directly — no file
/// reads, no blake3 work. Any change to any file falls back to the canonical
/// [`crate::core::hash::compute_sources_hash`] (which reads + hashes everything)
/// and refreshes the memo. The output is always equivalent to the canonical
/// function; the memo only elides redundant reads on no-change runs.
pub fn sources_hash(sources: &[PathBuf]) -> anyhow::Result<String> {
    let mut sorted: Vec<&PathBuf> = sources.iter().collect();
    sorted.sort();

    let memo = read_per_file_memo();
    let mut current: Vec<(String, u64, u64)> = Vec::with_capacity(sorted.len());
    let mut all_match = !memo.entries.is_empty() && memo.aggregate.is_some();
    for source in &sorted {
        let metadata =
            fs::metadata(source).map_err(|e| anyhow::anyhow!("failed to stat source {}: {e}", source.display()))?;
        let mtime_nanos = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let size = metadata.len();
        let path_str = source.to_string_lossy().to_string();
        if all_match {
            match memo.entries.get(&path_str) {
                Some((m, s)) if *m == mtime_nanos && *s == size => {}
                _ => all_match = false,
            }
        }
        current.push((path_str, mtime_nanos, size));
    }

    if all_match
        && current.len() == memo.entries.len()
        && let Some(agg) = memo.aggregate
    {
        return Ok(agg);
    }

    let aggregate = crate::core::hash::compute_sources_hash(sources)?;
    let _ = write_per_file_memo(&current, &aggregate);
    Ok(aggregate)
}

struct PerFileMemo {
    aggregate: Option<String>,
    entries: std::collections::HashMap<String, (u64, u64)>,
}

fn read_per_file_memo() -> PerFileMemo {
    let path = Path::new(CACHE_DIR).join(PER_FILE_CACHE_NAME);
    let Ok(content) = fs::read_to_string(&path) else {
        return PerFileMemo {
            aggregate: None,
            entries: std::collections::HashMap::new(),
        };
    };
    let mut aggregate: Option<String> = None;
    let mut entries = std::collections::HashMap::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("aggregate\t") {
            aggregate = Some(rest.to_string());
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let mtime_nanos = parts[1].parse::<u64>().unwrap_or(0);
        let size = parts[2].parse::<u64>().unwrap_or(0);
        entries.insert(parts[0].to_string(), (mtime_nanos, size));
    }
    PerFileMemo { aggregate, entries }
}

fn write_per_file_memo(entries: &[(String, u64, u64)], aggregate: &str) -> anyhow::Result<()> {
    let dir = Path::new(CACHE_DIR);
    fs::create_dir_all(dir)?;
    let mut content = format!("aggregate\t{aggregate}\n");
    for (path, mtime, size) in entries {
        content.push_str(&format!("{path}\t{mtime}\t{size}\n"));
    }
    fs::write(dir.join(PER_FILE_CACHE_NAME), content)?;
    Ok(())
}

/// Validate a crate name before using it as a filesystem path component.
///
/// Returns an error if the name contains path separators, NUL bytes, `..`,
/// or is a bare `.` — any of which could be used to escape the cache directory.
pub fn validate_cache_crate_name(crate_name: &str) -> anyhow::Result<()> {
    if crate_name.contains('\0') {
        anyhow::bail!("invalid crate name for cache: NUL byte not allowed in {crate_name:?}");
    }
    if crate_name.contains('/') || crate_name.contains('\\') {
        anyhow::bail!("invalid crate name for cache: path separator not allowed in {crate_name:?}");
    }
    if crate_name == ".." || crate_name == "." {
        anyhow::bail!("invalid crate name for cache: {crate_name:?} is not a valid crate name");
    }
    Ok(())
}

/// Return the per-crate IR cache directory, e.g. `.alef/<crate_name>/`.
fn ir_cache_dir(crate_name: &str) -> PathBuf {
    Path::new(CACHE_DIR).join(crate_name)
}

/// Check if cached IR is still valid for the given crate.
pub fn is_ir_cached(crate_name: &str, source_hash: &str) -> bool {
    let dir = ir_cache_dir(crate_name);
    let hash_path = dir.join("ir.hash");
    let ir_path = dir.join("ir.json");
    if !ir_path.exists() {
        return false;
    }
    match fs::read_to_string(&hash_path) {
        Ok(cached) => cached.trim() == source_hash,
        Err(_) => false,
    }
}

/// Read cached IR for the given crate.
pub fn read_cached_ir(crate_name: &str) -> anyhow::Result<crate::core::ir::ApiSurface> {
    let ir_path = ir_cache_dir(crate_name).join("ir.json");
    let content = fs::read_to_string(&ir_path)?;
    Ok(serde_json::from_str(&content)?)
}

/// Write IR to cache for the given crate.
pub fn write_ir_cache(crate_name: &str, api: &crate::core::ir::ApiSurface, source_hash: &str) -> anyhow::Result<()> {
    let cache_dir = ir_cache_dir(crate_name);
    fs::create_dir_all(&cache_dir)?;
    fs::write(cache_dir.join("ir.json"), serde_json::to_string_pretty(api)?)?;
    fs::write(cache_dir.join("ir.hash"), source_hash)?;
    Ok(())
}

/// Return a string representing the running alef binary's identity: mtime_nanos + file size.
/// Used to salt cache keys so that a locally-rebuilt binary always invalidates stale caches.
fn binary_identity() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| fs::metadata(&p).ok())
        .map(|m| {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            format!("{mtime}:{}", m.len())
        })
        .unwrap_or_default()
}

/// Compute hash for a language's output (IR + language-specific config + binary identity).
pub fn compute_lang_hash(ir_json: &str, lang: &str, config_toml: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(lang.as_bytes());
    hasher.update(config_toml.as_bytes());
    hasher.update(binary_identity().as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Per-crate hashes directory: `.alef/<crate>/hashes/`.
fn hashes_dir(crate_name: &str) -> PathBuf {
    ir_cache_dir(crate_name).join("hashes")
}

/// Check if a language's output is cached for the given crate.
/// Returns false if the hash doesn't match OR if any previously-generated
/// output files are missing from disk.
pub fn is_lang_cached(crate_name: &str, lang: &str, lang_hash: &str) -> bool {
    let dir = hashes_dir(crate_name);
    let hash_path = dir.join(format!("{lang}.hash"));
    let manifest_path = dir.join(format!("{lang}.manifest"));
    match fs::read_to_string(&hash_path) {
        Ok(cached) => {
            if cached.trim() != lang_hash {
                return false;
            }
            outputs_exist(&manifest_path)
        }
        Err(_) => false,
    }
}

/// Write language hash and output file manifest for the given crate.
pub fn write_lang_hash(crate_name: &str, lang: &str, lang_hash: &str, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{lang}.hash")), lang_hash)?;
    write_manifest(&dir.join(format!("{lang}.manifest")), output_paths)?;
    Ok(())
}

/// Replace a language manifest after every generation phase has contributed
/// its files. The language hash itself remains unchanged.
pub fn write_lang_manifest(crate_name: &str, lang: &str, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    fs::create_dir_all(&dir)?;
    write_manifest(&dir.join(format!("{lang}.manifest")), output_paths)
}

pub fn read_lang_manifest(crate_name: &str, lang: &str) -> Vec<PathBuf> {
    let manifest_path = hashes_dir(crate_name).join(format!("{lang}.manifest"));
    match fs::read_to_string(manifest_path) {
        Ok(content) => content
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Replace the crate-wide scaffold-ownership manifest with every path the
/// current run's scaffold pass emitted, deliberately including
/// `generated_header: false` seeds (`composer.json`, `package.json`, ...) that
/// carry no `alef:hash:` marker and are therefore invisible to
/// [`write_lang_manifest`]'s `carries_alef_marker()` filter.
///
/// This is the sole durable record that lets `sweep_manifest_orphans`'s
/// unmarkable-manifest route (see `path_is_reclaimable` in
/// `generate/orphans.rs`) reclaim a manifest a later run stops emitting (e.g. a
/// co-located/split PHP layout toggle that drops a second `composer.json`), and
/// it doubles as the current-run "keep" evidence that stops a manifest still
/// being written from ever being mistaken for an orphan of itself.
///
/// Crate-scoped rather than per-language like [`write_lang_manifest`] because
/// `scaffold()` returns a flat, unpartitioned file list; callers that only run
/// scaffold for a `--lang` subset must not call this, or the write here would
/// clobber the recorded paths for every other language's manifests.
pub fn write_scaffold_manifest(crate_name: &str, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    fs::create_dir_all(&dir)?;
    write_manifest(&dir.join("scaffold-ownership.manifest"), output_paths)
}

/// Read the previous run's scaffold-ownership manifest written by
/// [`write_scaffold_manifest`]. Empty when scaffold has never run for this
/// crate under this mechanism (including every run before this manifest was
/// introduced) -- callers must tolerate an empty result as "no known prior
/// scaffold state" rather than "nothing was ever scaffolded".
pub fn read_scaffold_manifest(crate_name: &str) -> Vec<PathBuf> {
    let manifest_path = hashes_dir(crate_name).join("scaffold-ownership.manifest");
    match fs::read_to_string(manifest_path) {
        Ok(content) => content
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Repo-scoped (rooted at `base_dir`, not crate-scoped) durable record of
/// every path alef owns whose format cannot carry an `alef:hash:` marker.
///
/// **Committed to git on purpose.** For every format that can carry a comment
/// the marker is the proof of ownership and it travels in the repository; for
/// `package.json`, `*.jar` and friends there is no such place to put it, so the
/// proof has to live in a separate file — and that file has to travel too. The
/// pre-#80 record lived at `.alef/scaffold-owned-paths.manifest`, inside the
/// directory alef writes into every consumer's `.gitignore` itself
/// (`cli::pipeline::extract::gitignore::ensure_gitignore`). That made ownership
/// a property of a particular developer's disk: a fresh clone and a warm
/// machine answered differently for the same commit, so CI refused writes a
/// developer's machine permitted. Sitting at the repo root outside `.alef/`,
/// this file is picked up by an ordinary `git add` and every checkout of a
/// commit agrees about what alef owns.
///
/// Deliberately additive and never replaced wholesale, unlike
/// [`write_scaffold_manifest`]'s per-crate, per-run snapshot: the write-time
/// ownership guard in `write_scaffold_files_report` has no crate name in
/// scope (it writes plain scaffold/readme/e2e/docs output keyed only by
/// `base_dir`) and is invoked incrementally from many independent commands
/// (readme, e2e regen, version sync, ...), so each call must extend the
/// record without erasing paths a different call already proved ownership
/// of. Rooted at `base_dir` rather than the process CWD so parallel tests
/// (each with their own tempdir `base_dir`) never share, and race on, the
/// same manifest file. ~keep
const OWNERSHIP_MANIFEST: &str = ".alef-ownership.toml";

/// The pre-#80 location of the same record, under the gitignored `.alef/` cache.
///
/// Still *read* (unioned with [`OWNERSHIP_MANIFEST`]) and never written. A
/// working copy that established ownership under an older alef keeps it, so
/// upgrading does not turn every unmarkable file in every existing consumer
/// repo into a refusal at once; the entry migrates into the committed manifest
/// the first time alef performs an authorised write of that path. Dropping the
/// read outright would be correct in the abstract and a mass outage in
/// practice. ~keep
const LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST: &str = "scaffold-owned-paths.manifest";

/// Preamble written above the path list.
///
/// Addressed at a human reading a `git diff` who has no reason to know what the
/// file is for: without it the natural reaction to a mystery dotfile is to
/// gitignore it, which restores exactly the bug this file exists to fix. ~keep
const OWNERSHIP_MANIFEST_HEADER: &str = "\
# alef ownership record -- COMMIT THIS FILE, do not add it to .gitignore.
#
# Lists the alef-generated paths whose format cannot carry an `alef:hash:`
# provenance marker (`package.json`, `*.jar`, ...). Every other format proves
# alef's ownership from the marker in the file itself and never appears here.
# Without this list committed, a fresh clone cannot tell an alef-generated
# `package.json` from a hand-written one and refuses to regenerate it.
#
# Ownership is a fact about history, not about content: a path lands here only
# because alef created the file, or because a human ran `alef adopt` on it.
# Nothing here is inferred by comparing bytes against generated output -- a
# hand-written file that happens to match must never be claimed. Do not hand-add
# entries; run `alef adopt <path>`, read the diff it prints, and let it write.
";

/// Normalize `path` to a `base_dir`-relative key before it is used to read or
/// write the owned-paths manifest.
///
/// Production callers of [`record_scaffold_owned_path`] / [`is_scaffold_owned_path`]
/// do not agree on how they spell `base_dir`: most `bin_cli` commands pass
/// `std::env::current_dir()?` (absolute), while `version_regen.rs`'s regen
/// helpers pass `PathBuf::from(".")` (relative) -- both name the same
/// directory, but `base_dir.join(&file.path)` produces textually different
/// strings from each (`/abs/repo/packages/java/pom.xml` vs
/// `./packages/java/pom.xml`). Storing and looking up that raw joined string
/// meant a record written by one caller was invisible to a lookup from the
/// other: `is_scaffold_owned_path` read as permanently `false` for any file
/// whose write-time caller and check-time caller happened to spell `base_dir`
/// differently, which in practice is most real cross-command sequences (e.g.
/// `alef all` establishes ownership, a later `alef version` bump checks it).
/// Stripping `base_dir` back off before keying makes the record depend only
/// on `file.path`, which every caller already agrees on. Falls back to the
/// path as given if it is not actually rooted at `base_dir` (should not
/// happen in practice, since every caller builds `path` via
/// `base_dir.join(...)`, but a mismatched pair must degrade to "some key"
/// rather than panic). ~keep
fn scaffold_owned_path_key(base_dir: &Path, path: &Path) -> String {
    path.strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[derive(serde::Deserialize)]
struct OwnershipManifest {
    #[serde(default)]
    owned_paths: Vec<String>,
}

fn ownership_manifest_path(base_dir: &Path) -> PathBuf {
    base_dir.join(OWNERSHIP_MANIFEST)
}

/// Read the committed record, treating an unreadable or unparseable file as
/// empty.
///
/// Degrading to "alef owns nothing" is the safe direction on both sides: the
/// guard then refuses rather than clobbers, and nothing is silently claimed on
/// the strength of a file we could not actually parse. A hard error here would
/// instead take down every generate in a repo where someone hand-edited the
/// manifest into invalid TOML. ~keep
fn read_committed_owned_paths(base_dir: &Path) -> Vec<String> {
    fs::read_to_string(ownership_manifest_path(base_dir))
        .ok()
        .and_then(|content| toml::from_str::<OwnershipManifest>(&content).ok())
        .map(|manifest| manifest.owned_paths)
        .unwrap_or_default()
}

fn read_legacy_owned_paths(base_dir: &Path) -> Vec<String> {
    let manifest_path = base_dir.join(CACHE_DIR).join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST);
    fs::read_to_string(manifest_path)
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Render the manifest by hand rather than through `toml::to_string`.
///
/// This file is read in `git diff` far more often than by a parser, and a
/// serializer is free to emit the array inline on one line. Adopting a single
/// path would then rewrite the whole line and show as a wholesale replacement,
/// which is precisely the shape that hides an unintended ownership claim from a
/// reviewer. One path per line makes every claim its own `+` line. ~keep
fn render_ownership_manifest(paths: &[String]) -> String {
    let mut content = String::from(OWNERSHIP_MANIFEST_HEADER);
    content.push_str("\nowned_paths = [\n");
    for path in paths {
        content.push_str("  \"");
        content.push_str(&path.replace('\\', "\\\\").replace('"', "\\\""));
        content.push_str("\",\n");
    }
    content.push_str("]\n");
    content
}

/// Record `path` (relative to `base_dir`, or already `base_dir`-joined -- see
/// [`scaffold_owned_path_key`]) as alef-owned, in the committed
/// [`OWNERSHIP_MANIFEST`].
///
/// The write-time guard in `write_scaffold_files_report` consults this for
/// extensions it cannot stamp with an `alef:hash:` marker (`.json`, `.jar`,
/// ...) to distinguish "alef legitimately wrote this before" from "this
/// pre-existed alef and must not be silently claimed." Idempotent: a path
/// already present is left alone, so a converged tree never rewrites the file
/// and never produces a spurious diff.
///
/// Callers must only reach this having established ownership *historically* --
/// alef created the file, or `alef adopt` obtained a human's consent for it.
/// Calling it because the bytes on disk happen to equal this run's output turns
/// a coincidence into a permanent, committed claim over a file nobody adopted;
/// see `cli::pipeline::generate::write::stamp_for_adoption` for the incident
/// that settles why byte-equality is not evidence. ~keep
pub fn record_scaffold_owned_path(base_dir: &Path, path: &Path) -> anyhow::Result<()> {
    record_scaffold_owned_paths(base_dir, std::slice::from_ref(&path))
}

/// Record every path in `paths` as alef-owned in one read-modify-write.
///
/// Semantically identical to calling [`record_scaffold_owned_path`] once per path,
/// which is exactly why it exists: that function reads, parses, re-renders and
/// rewrites the whole manifest per call, so adopting a batch through it costs
/// O(n) manifest parses over an O(n)-sized file — quadratic, and `alef adopt`
/// now has to clear ~12k unmarkable paths in a single consumer-repo migration.
/// One parse and one write for the whole batch makes that linear. The
/// per-path entry point delegates here rather than the reverse so there is a
/// single copy of the locking and rendering logic. ~keep
pub fn record_scaffold_owned_paths(base_dir: &Path, paths: &[&Path]) -> anyhow::Result<()> {
    // Serialised because this is a read-modify-write of one file and
    // `write_files_report` calls it from a rayon `par_iter`: two threads that both
    // observe the pre-write list and then both write it lose one entry, and a lost
    // entry is a path alef silently stops owning — a refusal on the next run, in CI,
    // for a file alef itself created. The old gitignored record had the same race and
    // could be repaired by rerunning locally; a committed one gets the wrong answer
    // captured in a commit instead. Cross-*process* concurrency in one repo is not a
    // supported mode for any of this module's caches. ~keep
    static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());

    if paths.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(base_dir)?;
    let mut recorded: std::collections::BTreeSet<String> = read_committed_owned_paths(base_dir).into_iter().collect();
    let mut added = false;
    for path in paths {
        added |= recorded.insert(scaffold_owned_path_key(base_dir, path));
    }
    if !added {
        return Ok(());
    }
    let is_new_manifest = !ownership_manifest_path(base_dir).exists();
    let ordered: Vec<String> = recorded.into_iter().collect();
    fs::write(ownership_manifest_path(base_dir), render_ownership_manifest(&ordered))?;
    if is_new_manifest {
        tracing::info!(
            manifest = %OWNERSHIP_MANIFEST,
            "created the alef ownership record: commit it, or a fresh clone cannot regenerate \
             the unmarkable files listed in it"
        );
    }
    Ok(())
}

/// True when `path` was previously recorded by [`record_scaffold_owned_path`]
/// for this `base_dir`.
///
/// Reads the committed [`OWNERSHIP_MANIFEST`] unioned with the legacy
/// gitignored record (see [`LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST`]). Once the
/// committed manifest is in the repository this answers identically on a fresh
/// clone and on a warm machine, which is the whole point of moving it; the
/// legacy half is the only remaining source of machine-local divergence and it
/// can only ever say `true` where the old code already did. When neither knows
/// the path the answer is `false` and the write-time guard refuses rather than
/// risk clobbering foreign content. ~keep
pub fn is_scaffold_owned_path(base_dir: &Path, path: &Path) -> bool {
    let key = scaffold_owned_path_key(base_dir, path);
    read_committed_owned_paths(base_dir)
        .iter()
        .chain(read_legacy_owned_paths(base_dir).iter())
        .any(|existing| *existing == key)
}

/// Repo-scoped (rooted at `base_dir`) local record of the array *values*
/// alef's own generator proposed for a TOML merge target, per dotted
/// key path, on the most recent successful merge -- e.g.
/// `{"poly.toml": {"discovery.exclude": ["target/**", "docs/snippets/**"]}}`.
///
/// This is the provenance data [`merge_managed_toml`]'s prune step needs to
/// answer "did alef itself, in a past run, propose this exact value" without
/// guessing from the value's text alone: a value present in `existing` that
/// merely *equals* something alef's current template happens to emit is not
/// evidence of authorship (a consumer's own `[workspace.poly] exclude` entry
/// can coincide), but a value that was captured here -- straight from alef's
/// own generated output, before any merge with consumer content -- genuinely
/// was alef's proposal. A value the consumer configures via
/// `[workspace.poly] exclude` (or `file_safety_exclude`) is echoed back into
/// the generator's own output on every run for as long as it stays
/// configured, so it keeps reappearing here too and is never a prune
/// candidate; it only becomes one if the consumer removes it from their own
/// config, at which point pruning it matches their own subsequent intent.
///
/// Deliberately keyed by the merge target's *relative* path (`"poly.toml"`),
/// not the `base_dir`-joined absolute one, so the record does not depend on
/// how a given invocation happened to express `base_dir`.
///
/// `.alef/` is gitignored and machine-local: a fresh clone or a wiped cache
/// has no record for any key path, so [`merge_managed_toml`]'s prune step
/// finds nothing to compare against and removes nothing -- the same
/// degrade-safely-to-no-op contract as [`is_scaffold_owned_path`], and for
/// the same reason: this can only ever prevent *future* drift from
/// accumulating starting at the first run that establishes a baseline in a
/// given working copy, never retroactively clean up values that went stale
/// before that baseline existed. ~keep
const TOML_MERGE_PROVENANCE_MANIFEST: &str = "toml-merge-provenance.json";

type TomlMergeProvenance = std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>;

fn read_toml_merge_provenance_file(base_dir: &Path) -> TomlMergeProvenance {
    let manifest_path = base_dir.join(CACHE_DIR).join(TOML_MERGE_PROVENANCE_MANIFEST);
    fs::read_to_string(manifest_path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// Read the previously recorded array values for every key path in
/// `relative_path` (e.g. `"poly.toml"`). Empty when nothing was ever
/// recorded for this path in this working copy -- callers must treat that as
/// "no known prior proposal," never as "alef proposed no arrays."
pub fn read_toml_merge_provenance(
    base_dir: &Path,
    relative_path: &Path,
) -> std::collections::BTreeMap<String, Vec<String>> {
    read_toml_merge_provenance_file(base_dir)
        .remove(&relative_path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Replace the recorded array values for `relative_path` with
/// `arrays_by_key_path` -- this run's freshly generated content, captured
/// before merging with consumer content -- for the next run's comparison.
/// Other merge targets' records are left untouched.
pub fn write_toml_merge_provenance(
    base_dir: &Path,
    relative_path: &Path,
    arrays_by_key_path: &std::collections::BTreeMap<String, Vec<String>>,
) -> anyhow::Result<()> {
    let dir = base_dir.join(CACHE_DIR);
    fs::create_dir_all(&dir)?;
    let mut all = read_toml_merge_provenance_file(base_dir);
    all.insert(relative_path.to_string_lossy().into_owned(), arrays_by_key_path.clone());
    let manifest_path = dir.join(TOML_MERGE_PROVENANCE_MANIFEST);
    fs::write(&manifest_path, serde_json::to_string_pretty(&all)?)?;
    Ok(())
}

/// Compute hash for a generation stage (stubs, docs, readme, scaffold, e2e).
/// `extra` allows including additional content (e.g., fixture files for e2e).
/// The alef binary's identity is included so that locally rebuilt binaries
/// always invalidate stale caches without requiring a version bump.
pub fn compute_stage_hash(ir_json: &str, stage: &str, config_toml: &str, extra: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(stage.as_bytes());
    hasher.update(config_toml.as_bytes());
    if !extra.is_empty() {
        hasher.update(extra);
    }
    hasher.update(binary_identity().as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Check if a stage's output is cached for the given crate.
/// Returns false if the hash doesn't match OR if any previously-generated
/// output files are missing from disk.
pub fn is_stage_cached(crate_name: &str, stage: &str, stage_hash: &str) -> bool {
    let dir = hashes_dir(crate_name);
    let hash_path = dir.join(format!("{stage}.hash"));
    let manifest_path = dir.join(format!("{stage}.manifest"));
    match fs::read_to_string(&hash_path) {
        Ok(cached) => {
            if cached.trim() != stage_hash {
                return false;
            }
            outputs_exist(&manifest_path)
        }
        Err(_) => false,
    }
}

/// Read the manifest of output paths previously written for the given stage.
///
/// Returns an empty `Vec` when the manifest does not exist (either the stage
/// has never been generated for this crate, or the cache predates the manifest
/// format introduced in 0.18.1). Callers should use this to repopulate
/// `current_gen_paths` on a cache hit so the orphan-cleanup pass does not
/// delete files that the previous run wrote but the current run skipped.
pub fn read_stage_paths(crate_name: &str, stage: &str) -> Vec<PathBuf> {
    let dir = hashes_dir(crate_name);
    let manifest_path = dir.join(format!("{stage}.manifest"));
    match fs::read_to_string(&manifest_path) {
        Ok(content) => content
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Write stage hash and output file manifest for the given crate.
pub fn write_stage_hash(
    crate_name: &str,
    stage: &str,
    stage_hash: &str,
    output_paths: &[PathBuf],
) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{stage}.hash")), stage_hash)?;
    write_manifest(&dir.join(format!("{stage}.manifest")), output_paths)?;
    Ok(())
}

/// Write a manifest of output file paths (one per line).
fn write_manifest(manifest_path: &Path, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let mut paths: Vec<_> = output_paths.iter().map(|p| p.to_string_lossy()).collect();
    paths.sort_unstable();
    paths.dedup();
    let mut content = paths.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    fs::write(manifest_path, content)?;
    Ok(())
}

/// Check that all files listed in a manifest exist on disk.
/// Returns true if the manifest is missing (backwards compat with old caches)
/// or if all listed files exist. Returns false if any file is missing.
fn outputs_exist(manifest_path: &Path) -> bool {
    match fs::read_to_string(manifest_path) {
        Ok(content) => {
            let mut paths = content.lines().filter(|line| !line.is_empty()).peekable();
            paths.peek().is_some() && paths.all(|line| Path::new(line).exists())
        }
        Err(_) => true,
    }
}

/// Hash all files in a directory recursively (for e2e fixture hashing).
pub fn hash_directory(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let mut hasher = blake3::Hasher::new();
    if dir.exists() {
        let mut entries: Vec<_> = walkdir(dir)?;
        entries.sort();
        for path in entries {
            let content = fs::read(&path)?;
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(&content);
        }
    }
    Ok(hasher.finalize().as_bytes().to_vec())
}

fn walkdir(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

/// Blake3 hash of a content string.
pub fn hash_content(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// Store generation content hashes: Vec of (path_display, content_hash).
///
/// Call this with pre-computed hashes — use [`hash_content`] on each file's
/// content string before calling.  Stored before writing to disk so hashes
/// reflect pure codegen output, independent of any on-disk formatter.
pub fn write_generation_hashes(name: &str, hashes: &[(String, String)]) -> anyhow::Result<()> {
    let dir = Path::new(CACHE_DIR).join("hashes");
    fs::create_dir_all(&dir)?;
    let lines: Vec<String> = hashes.iter().map(|(p, h)| format!("{p}\t{h}")).collect();
    fs::write(dir.join(format!("{name}.output_hashes")), lines.join("\n"))?;
    Ok(())
}

/// Load stored generation hashes as `HashMap<path, hash>`.
pub fn read_generation_hashes(name: &str) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let path = Path::new(CACHE_DIR)
        .join("hashes")
        .join(format!("{name}.output_hashes"));
    let content = fs::read_to_string(&path)?;
    Ok(content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .map(|(p, h)| (p.to_string(), h.to_string()))
        .collect())
}

/// Clear cache.
pub fn clear_cache() -> anyhow::Result<()> {
    let cache_dir = Path::new(CACHE_DIR);
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

/// Show cache status information.
pub fn show_status() {
    let cache_dir = Path::new(CACHE_DIR);
    if !cache_dir.exists() {
        crate::bin_cli::output::line("No cache directory.");
        return;
    }

    crate::bin_cli::output::line("Cache directory: .alef/");

    let ir_path = cache_dir.join("ir.json");
    if ir_path.exists() {
        if let Ok(meta) = fs::metadata(&ir_path) {
            crate::bin_cli::output::line(format!("  ir.json: {} bytes", meta.len()));
        }
    } else {
        crate::bin_cli::output::line("  ir.json: not cached");
    }

    let hashes_dir = cache_dir.join("hashes");
    if hashes_dir.exists() {
        if let Ok(entries) = fs::read_dir(&hashes_dir) {
            let langs: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str().map(String::from)))
                .collect();
            if langs.is_empty() {
                crate::bin_cli::output::line("  language hashes: none");
            } else {
                crate::bin_cli::output::line(format!("  language hashes: {}", langs.join(", ")));
            }
        }
    } else {
        crate::bin_cli::output::line("  language hashes: none");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_with_ordered_entries(entries: &[(&str, &str)]) -> crate::core::ir::ApiSurface {
        let mut api = crate::core::ir::ApiSurface {
            crate_name: "sample_crate".to_string(),
            ..Default::default()
        };
        for (name, path) in entries {
            api.excluded_type_paths.insert((*name).to_string(), (*path).to_string());
            api.excluded_trait_names.insert((*name).to_string());
        }
        api
    }

    #[test]
    fn validate_cache_crate_name_accepts_normal_names() {
        validate_cache_crate_name("my-lib").unwrap();
        validate_cache_crate_name("sample_crate").unwrap();
        validate_cache_crate_name("sample_markdown").unwrap();
    }

    #[test]
    fn validate_cache_crate_name_rejects_path_separators() {
        assert!(validate_cache_crate_name("../escape").is_err());
        assert!(validate_cache_crate_name("foo/bar").is_err());
        assert!(validate_cache_crate_name("foo\\bar").is_err());
    }

    #[test]
    fn validate_cache_crate_name_rejects_dot_aliases() {
        assert!(validate_cache_crate_name("..").is_err());
        assert!(validate_cache_crate_name(".").is_err());
    }

    #[test]
    fn validate_cache_crate_name_rejects_nul_byte() {
        assert!(validate_cache_crate_name("foo\0bar").is_err());
    }

    #[test]
    fn ir_cache_dir_scopes_by_crate_name() {
        assert_eq!(ir_cache_dir("crate-a"), Path::new(CACHE_DIR).join("crate-a"));
        assert_eq!(ir_cache_dir("crate-b"), Path::new(CACHE_DIR).join("crate-b"));
        assert_ne!(ir_cache_dir("crate-a"), ir_cache_dir("crate-b"));
    }

    #[test]
    fn repeated_ir_serialization_preserves_cache_and_provenance_hashes() {
        let first = api_with_ordered_entries(&[
            ("Gamma", "sample_crate::gamma::Gamma"),
            ("Alpha", "sample_crate::alpha::Alpha"),
            ("Beta", "sample_crate::beta::Beta"),
        ]);
        let second = api_with_ordered_entries(&[
            ("Beta", "sample_crate::beta::Beta"),
            ("Gamma", "sample_crate::gamma::Gamma"),
            ("Alpha", "sample_crate::alpha::Alpha"),
        ]);

        let first_json = serde_json::to_string_pretty(&first).expect("serialize first IR");
        let second_json = serde_json::to_string_pretty(&second).expect("serialize second IR");
        let generated = "// auto-generated by alef\npub fn sample() {}\n";
        let first_cache_hash = compute_lang_hash(&first_json, "sample", "[sample]\n");
        let second_cache_hash = compute_lang_hash(&second_json, "sample", "[sample]\n");
        let first_file_hash = crate::core::hash::compute_file_hash(&first_cache_hash, generated);
        let second_file_hash = crate::core::hash::compute_file_hash(&second_cache_hash, generated);

        assert_eq!(first_json, second_json);
        assert_eq!(first_cache_hash, second_cache_hash);
        assert_eq!(first_file_hash, second_file_hash);
        assert_eq!(
            crate::core::hash::inject_hash_line(generated, &first_file_hash),
            crate::core::hash::inject_hash_line(generated, &second_file_hash)
        );
    }

    #[test]
    fn manifest_is_sorted_deduplicated_and_newline_terminated() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manifest = directory.path().join("rust.manifest");
        let alpha = directory.path().join("alpha.rs");
        let beta = directory.path().join("beta.rs");

        write_manifest(&manifest, &[beta.clone(), alpha.clone(), beta.clone()]).expect("write manifest");

        let content = std::fs::read_to_string(manifest).expect("read manifest");
        assert_eq!(content, format!("{}\n{}\n", alpha.display(), beta.display()));
    }

    #[test]
    fn empty_manifest_is_not_a_cache_hit() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manifest = directory.path().join("rust.manifest");
        std::fs::write(&manifest, "").expect("write empty manifest");

        assert!(!outputs_exist(&manifest));
    }

    /// Serialize tests that mutate the process-global current directory, mirroring
    /// the lock in `cli::pipeline::version_tests` -- `write_scaffold_manifest`/
    /// `read_scaffold_manifest` resolve `.alef/<crate>/hashes/` relative to CWD,
    /// so concurrent tempdir-based tests below would race without it. ~keep
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// `write_scaffold_manifest` must round-trip through `read_scaffold_manifest`,
    /// sorted and deduplicated like every other manifest. This is the durable
    /// record `sweep_manifest_orphans`'s unmarkable-manifest route depends on to
    /// know a `composer.json`/`package.json` path was scaffold's on a prior run --
    /// without it, `read_scaffold_manifest` (which does not exist on unfixed code)
    /// cannot be called at all.
    #[test]
    fn scaffold_manifest_round_trips_through_write_and_read() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(tmp.path()).expect("chdir into tempdir");

        let composer = tmp.path().join("packages/php/composer.json");
        let cargo_toml = tmp.path().join("Cargo.toml");
        let write_result = write_scaffold_manifest("sample-crate", &[composer.clone(), cargo_toml.clone()]);
        let read_back = read_scaffold_manifest("sample-crate");

        let _ = std::env::set_current_dir(&original_cwd);
        write_result.expect("write scaffold manifest");
        assert_eq!(
            read_back,
            vec![cargo_toml, composer],
            "manifest must round-trip both paths in sorted order"
        );
    }

    /// A crate that has never had scaffold run under this mechanism (including
    /// every run before it existed) must read back empty rather than erroring --
    /// callers treat an empty result as "no known prior scaffold state", never as
    /// proof nothing was ever scaffolded.
    #[test]
    fn scaffold_manifest_reads_empty_when_never_written() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(tmp.path()).expect("chdir into tempdir");

        let read_back = read_scaffold_manifest("never-scaffolded-crate");

        let _ = std::env::set_current_dir(&original_cwd);
        assert_eq!(read_back, Vec::<PathBuf>::new());
    }

    /// End-to-end regression for the `composer.json` orphan observed in a consumer repo: proves the
    /// `write_scaffold_manifest`/`read_scaffold_manifest` wiring is what lets
    /// `sweep_manifest_orphans` reclaim an unmarkable manifest a later run stops
    /// emitting. Before this manifest existed, nothing ever recorded
    /// `composer.json`'s path -- `write_lang_manifest` and every
    /// `generate-{lang}-ownership` stage filter scaffold paths through
    /// `carries_alef_marker()`, which `composer.json` never satisfies (it is
    /// emitted with `generated_header: false`) -- so `sweep_manifest_orphans` was
    /// always called with an empty `previous_paths` for this file and could never
    /// reach it, regardless of how permissive `path_is_reclaimable` is. On unfixed
    /// code, `previous_scaffold` here is empty (no prior-run record exists), so
    /// `sweep_manifest_orphans` skips `composer_json` entirely and `removed` is 0,
    /// failing the `assert_eq!(removed, 1, ...)` below.
    #[test]
    fn scaffold_manifest_wiring_lets_next_run_reclaim_dropped_manifest() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(tmp.path()).expect("chdir into tempdir");

        let package_dir = tmp.path().join("packages/php");
        std::fs::create_dir_all(&package_dir).expect("create package dir");
        let composer_json = package_dir.join("composer.json");
        std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("write composer.json");

        write_scaffold_manifest("sample-php", std::slice::from_ref(&composer_json)).expect("write manifest for run 1");

        let previous_scaffold = read_scaffold_manifest("sample-php");
        let keep = std::collections::HashSet::new();
        let removed =
            crate::cli::pipeline::sweep_manifest_orphans(&previous_scaffold, &keep, &[package_dir]).expect("sweep");

        let _ = std::env::set_current_dir(&original_cwd);
        assert_eq!(
            removed, 1,
            "composer.json recorded by run 1's manifest must be reclaimed in run 2"
        );
        assert!(!composer_json.exists(), "orphaned composer.json must be deleted");
    }

    #[test]
    fn scaffold_owned_path_round_trips_and_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target = base.join("packages/java/pom.xml");

        assert!(!is_scaffold_owned_path(base, &target), "must start unrecorded");

        record_scaffold_owned_path(base, &target).expect("record");
        record_scaffold_owned_path(base, &target).expect("record again (idempotent)");

        assert!(is_scaffold_owned_path(base, &target));
        let manifest = std::fs::read_to_string(base.join(OWNERSHIP_MANIFEST)).expect("read manifest");
        assert_eq!(
            manifest.matches("packages/java/pom.xml").count(),
            1,
            "recording the same path twice must not duplicate it, got:\n{manifest}"
        );
        assert!(
            !base.join(".alef").join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST).exists(),
            "the gitignored legacy record must no longer be written, got:\n{manifest}"
        );
    }

    /// The batch entry point must be indistinguishable in outcome from the per-path
    /// one — same entries, same order, same idempotence, and existing entries left
    /// alone — because it exists purely to collapse N manifest parses into one for a
    /// bulk `alef adopt`. If it ever diverges in *result*, the fast path is silently
    /// recording something different from what the reviewed path would have. ~keep
    #[test]
    fn batch_recording_matches_per_path_recording_entry_for_entry() {
        let batched = tempfile::tempdir().expect("tempdir");
        let one_at_a_time = tempfile::tempdir().expect("tempdir");
        let relatives = [
            "docs/snippets/python/api/z.md",
            "packages/node/package.json",
            "docs/snippets/python/api/a.md",
            "packages/java/pom.xml",
        ];

        record_scaffold_owned_path(batched.path(), &batched.path().join("pre/existing.json")).expect("seed");
        record_scaffold_owned_path(one_at_a_time.path(), &one_at_a_time.path().join("pre/existing.json"))
            .expect("seed");

        let joined: Vec<PathBuf> = relatives.iter().map(|rel| batched.path().join(rel)).collect();
        let refs: Vec<&Path> = joined.iter().map(PathBuf::as_path).collect();
        record_scaffold_owned_paths(batched.path(), &refs).expect("batch record");
        record_scaffold_owned_paths(batched.path(), &refs).expect("batch record again (idempotent)");
        for relative in relatives {
            record_scaffold_owned_path(one_at_a_time.path(), &one_at_a_time.path().join(relative)).expect("record");
        }

        assert_eq!(
            std::fs::read_to_string(batched.path().join(OWNERSHIP_MANIFEST)).expect("batched manifest"),
            std::fs::read_to_string(one_at_a_time.path().join(OWNERSHIP_MANIFEST)).expect("sequential manifest"),
        );
        for relative in relatives {
            assert!(is_scaffold_owned_path(batched.path(), &batched.path().join(relative)));
        }
        assert!(
            is_scaffold_owned_path(batched.path(), &batched.path().join("pre/existing.json")),
            "a batch must extend the record, never replace it"
        );
    }

    /// The record must be a file `git add` picks up, not one alef itself
    /// gitignores. `ensure_gitignore` writes `.alef/` into every consumer's
    /// `.gitignore` (`cli::pipeline::extract::gitignore`), so a record stored
    /// under that directory can never travel with the commit it describes --
    /// which is the entire #80 reproducibility hole. Asserting the location and
    /// the parseability together, because a committed file nobody can parse is
    /// worth no more than an ignored one. ~keep
    #[test]
    fn ownership_record_lives_outside_the_gitignored_cache_and_is_valid_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        record_scaffold_owned_path(base, &base.join("packages/typescript/package.json")).expect("record");

        let manifest_path = base.join(OWNERSHIP_MANIFEST);
        assert!(manifest_path.exists(), "the record must exist at the repo root");
        assert!(
            !manifest_path.starts_with(base.join(CACHE_DIR)),
            "the record must not live under the gitignored `{CACHE_DIR}` directory"
        );
        let content = std::fs::read_to_string(&manifest_path).expect("read manifest");
        let parsed: OwnershipManifest = toml::from_str(&content).expect("the record must be valid TOML");
        assert_eq!(parsed.owned_paths, vec!["packages/typescript/package.json".to_owned()]);
    }

    /// A fresh clone carries the committed record but no `.alef/` cache at all.
    /// Simulated by recording into one `base_dir` and reading the manifest back
    /// from a second, cache-less one -- the machine-local half of the answer is
    /// absent there by construction, so a `true` can only have come from the
    /// committed file.
    #[test]
    fn committed_record_answers_identically_on_a_cache_less_clone() {
        let warm = tempfile::tempdir().expect("tempdir warm");
        let clone = tempfile::tempdir().expect("tempdir clone");
        let relative = std::path::Path::new("packages/typescript/package.json");

        record_scaffold_owned_path(warm.path(), &warm.path().join(relative)).expect("record");
        std::fs::copy(
            warm.path().join(OWNERSHIP_MANIFEST),
            clone.path().join(OWNERSHIP_MANIFEST),
        )
        .expect("check out the committed record");

        assert!(
            !clone.path().join(CACHE_DIR).exists(),
            "the simulated clone must have no machine-local cache"
        );
        assert!(
            is_scaffold_owned_path(clone.path(), &clone.path().join(relative)),
            "a fresh clone must agree with the warm machine about what alef owns"
        );
    }

    /// A record written by a pre-#80 alef, which exists only under the
    /// gitignored cache, must keep working -- otherwise upgrading turns every
    /// unmarkable file in every existing consumer repo into a refusal at once.
    #[test]
    fn legacy_gitignored_record_is_still_honoured_for_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let relative = std::path::Path::new("packages/java/pom.xml");

        std::fs::create_dir_all(base.join(CACHE_DIR)).expect("create legacy cache dir");
        std::fs::write(
            base.join(CACHE_DIR).join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST),
            "packages/java/pom.xml\n",
        )
        .expect("seed legacy record");

        assert!(!base.join(OWNERSHIP_MANIFEST).exists(), "no committed record yet");
        assert!(is_scaffold_owned_path(base, &base.join(relative)));
    }

    /// An unparseable record must read as "alef owns nothing" rather than
    /// panicking or, far worse, being treated as ownership of everything.
    #[test]
    fn unparseable_ownership_record_claims_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        std::fs::write(base.join(OWNERSHIP_MANIFEST), "this is not = = valid toml [[[").expect("write junk");

        assert!(!is_scaffold_owned_path(
            base,
            &base.join("packages/typescript/package.json")
        ));
    }

    /// The record is itself a `.toml` file at the repo root, so `alef verify`'s walk
    /// reaches it. Its explanatory header must not read as a provenance marker
    /// ([`crate::core::hash::content_has_alef_marker`] matches the substrings
    /// "auto-generated by alef" / "Generated by alef" anywhere in the first ten lines):
    /// a file that claims to be alef-stamped but is outside the generated-file hash
    /// pipeline has no computable hash, so it would surface as permanently stale. The
    /// header is prose a human wrote and is easy to reword into a false positive, which
    /// is why this is pinned rather than left to care. ~keep
    #[test]
    fn ownership_record_header_does_not_read_as_a_provenance_marker() {
        let rendered = render_ownership_manifest(&["packages/typescript/package.json".to_owned()]);
        assert!(
            !crate::core::hash::content_has_alef_marker(&rendered),
            "the record's own header must not look like an alef provenance marker, got:\n{rendered}"
        );
    }

    /// A path containing a quote or a backslash (a Windows-spelled key, a perverse but
    /// legal filename) must survive the hand-rolled TOML writer. Escaping it wrongly
    /// produces a manifest that no longer parses, and an unparseable manifest reads as
    /// "alef owns nothing" -- so the failure would not be loud, it would quietly un-own
    /// every path in the repo at once. ~keep
    #[test]
    fn ownership_record_escapes_paths_that_need_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let awkward = "packages/we\"ird\\name.json";

        record_scaffold_owned_path(base, &base.join(awkward)).expect("record");
        record_scaffold_owned_path(base, &base.join("packages/plain.json")).expect("record plain");

        let content = std::fs::read_to_string(base.join(OWNERSHIP_MANIFEST)).expect("read manifest");
        let parsed: OwnershipManifest = toml::from_str(&content).expect("manifest must stay parseable");
        assert!(
            parsed.owned_paths.iter().any(|path| path == awkward),
            "the awkward path must round-trip unchanged, got: {:?}",
            parsed.owned_paths
        );
        assert!(is_scaffold_owned_path(base, &base.join(awkward)));
        assert!(
            is_scaffold_owned_path(base, &base.join("packages/plain.json")),
            "a bad escape must not take the rest of the record down with it"
        );
    }

    #[test]
    fn scaffold_owned_path_is_scoped_to_base_dir() {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let dir_b = tempfile::tempdir().expect("tempdir b");
        let target = std::path::PathBuf::from("packages/java/pom.xml");

        record_scaffold_owned_path(dir_a.path(), &dir_a.path().join(&target)).expect("record in a");

        assert!(!is_scaffold_owned_path(dir_b.path(), &dir_b.path().join(&target)));
    }

    /// Regression: a record written with an *absolute* `base_dir`
    /// (`std::env::current_dir()`, what most `bin_cli` commands pass) must
    /// still be found by a lookup that expresses `base_dir` *relatively*
    /// (`PathBuf::from(".")`, what `version_regen.rs`'s regen helpers pass)
    /// when both name the same directory -- and vice versa. Before
    /// `scaffold_owned_path_key` normalized the stored key back to
    /// `file.path`, the two representations produced different
    /// `base_dir.join(path)` strings for the same file, so
    /// `is_scaffold_owned_path` read as permanently `false` for any path
    /// whose owning write and later check happened to come from commands
    /// that spell `base_dir` differently -- which most real multi-command
    /// sequences do (e.g. `alef all` establishes ownership, a later
    /// `alef version` bump checks it), making the manifest effectively inert
    /// even though it was being written and read from the exact same file on
    /// disk the whole time.
    #[test]
    fn scaffold_owned_path_matches_across_absolute_and_relative_base_dir_spellings() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(tmp.path()).expect("chdir into tempdir");

        let absolute_base = std::env::current_dir().expect("absolute cwd");
        let relative_base = Path::new(".");
        let relative_target = relative_base.join("packages/java/pom.xml");

        let result = (|| -> anyhow::Result<(bool, bool)> {
            // Written as an absolute-`base_dir` caller (e.g. a `bin_cli` command) would.
            record_scaffold_owned_path(&absolute_base, &absolute_base.join("packages/java/pom.xml"))?;
            // Checked as a relative-`base_dir` caller (e.g. `version_regen.rs`) would.
            let found_from_relative = is_scaffold_owned_path(relative_base, &relative_target);
            // And the reverse direction: written relatively, checked absolutely.
            record_scaffold_owned_path(relative_base, &relative_base.join("packages/csharp/foo.csproj"))?;
            let found_from_absolute =
                is_scaffold_owned_path(&absolute_base, &absolute_base.join("packages/csharp/foo.csproj"));
            Ok((found_from_relative, found_from_absolute))
        })();

        let _ = std::env::set_current_dir(&original_cwd);
        let (found_from_relative, found_from_absolute) = result.expect("record/check round-trip");
        assert!(
            found_from_relative,
            "a record written with an absolute base_dir must be found by a relative-base_dir lookup"
        );
        assert!(
            found_from_absolute,
            "a record written with a relative base_dir must be found by an absolute-base_dir lookup"
        );
    }
}
