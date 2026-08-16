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
/// every path a write pass has confirmed alef legitimately wrote or reused.
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
const SCAFFOLD_OWNED_PATHS_MANIFEST: &str = "scaffold-owned-paths.manifest";

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

/// Record `path` (relative to `base_dir`, or already `base_dir`-joined -- see
/// [`scaffold_owned_path_key`]) as alef-owned.
///
/// The write-time guard in `write_scaffold_files_report` consults this for
/// extensions it cannot stamp with an `alef:hash:` marker (`.md`, `.json`,
/// `.xml`, ...) to distinguish "alef legitimately wrote this before" from
/// "this pre-existed alef and must not be silently claimed." Idempotent: a
/// path already present is left alone.
pub fn record_scaffold_owned_path(base_dir: &Path, path: &Path) -> anyhow::Result<()> {
    let dir = base_dir.join(CACHE_DIR);
    fs::create_dir_all(&dir)?;
    let manifest_path = dir.join(SCAFFOLD_OWNED_PATHS_MANIFEST);
    let key = scaffold_owned_path_key(base_dir, path);
    let mut paths = read_scaffold_owned_paths_raw(&manifest_path);
    if paths.iter().any(|existing| *existing == key) {
        return Ok(());
    }
    paths.push(key);
    paths.sort_unstable();
    paths.dedup();
    let mut content = paths.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    fs::write(&manifest_path, content)?;
    Ok(())
}

/// True when `path` was previously recorded by [`record_scaffold_owned_path`]
/// under this `base_dir`'s local `.alef/` cache.
///
/// `.alef/` is gitignored and machine-local, so a fresh clone or a
/// cache-less CI job always answers `false` here -- the write-time guard
/// treats that as "no durable evidence," refusing to overwrite rather than
/// risk clobbering foreign content. ~keep
pub fn is_scaffold_owned_path(base_dir: &Path, path: &Path) -> bool {
    let manifest_path = base_dir.join(CACHE_DIR).join(SCAFFOLD_OWNED_PATHS_MANIFEST);
    let key = scaffold_owned_path_key(base_dir, path);
    read_scaffold_owned_paths_raw(&manifest_path)
        .iter()
        .any(|existing| *existing == key)
}

fn read_scaffold_owned_paths_raw(manifest_path: &Path) -> Vec<String> {
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

    /// End-to-end regression for the crawlberg `composer.json` orphan: proves the
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

        write_scaffold_manifest("crawlberg-php", std::slice::from_ref(&composer_json))
            .expect("write manifest for run 1");

        let previous_scaffold = read_scaffold_manifest("crawlberg-php");
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
        let manifest =
            std::fs::read_to_string(base.join(".alef").join(SCAFFOLD_OWNED_PATHS_MANIFEST)).expect("read manifest");
        assert_eq!(
            manifest.lines().count(),
            1,
            "recording the same path twice must not duplicate it, got:\n{manifest}"
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
