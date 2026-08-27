//! Cache-key identity for every `.alef/` cache that can *skip* work.
//!
//! Split out of [`crate::cli::cache`] so the "what makes a cache entry stale?"
//! question lives in one bounded place. Three keys are computed here:
//! [`compute_ir_key`] (the extracted `ApiSurface`, consumed by
//! `cache::is_ir_cached`), [`compute_lang_hash`] (per target language, consumed by
//! `cache::is_lang_cached`) and [`compute_stage_hash`] (per generation stage —
//! stubs, docs, readme, scaffold, e2e — consumed by `cache::is_stage_cached`).
//!
//! # Why the key is a type and not a `String`
//!
//! Every key built here folds in the alef build identity, so no cache entry
//! survives an alef upgrade. That invariant held for the language and stage
//! caches and was silently missing from the IR cache, which was keyed inline in
//! `pipeline::extract` on `sources + crate version + config` alone: a newer alef
//! replayed an older alef's `ApiSurface` verbatim, and — because most
//! `ApiSurface` fields are `#[serde(default)]` — a field the older extractor
//! never wrote came back as its default rather than as an error. Both `alef
//! generate` and `alef verify` consume that surface, so the whole run reported
//! green over another release's extraction.
//!
//! [`CacheKey`]'s field is private to this module, so it cannot be constructed
//! anywhere else, and the three `is_*_cached` predicates accept nothing else.
//! A future cache therefore cannot gate a skip on a key that forgot the salt —
//! not because a test noticed, but because it does not compile. ~keep
//!
//! `cache::write_stage_hash` is the one deliberate exception, and takes `&str`:
//! the `*-ownership` stages and `breaking_changes`' signature baseline reuse it
//! purely as a named path manifest, storing a plain content hash they never read
//! back through `is_stage_cached`. Widening [`CacheKey`] to cover them would have
//! meant handing those call sites a constructor — exactly the escape hatch the
//! type exists to withhold. The invariant is enforced where it matters, on the
//! read side that actually decides whether work is skipped. ~keep
//!
//! # These are NOT the embedded `alef:hash:` stamp
//!
//! Generated files carry an `alef:hash:` value produced by
//! [`crate::core::hash::compute_file_hash`], a function of that one file's own
//! content and nothing else. It deliberately folds in *no* generation inputs:
//! when it did, one edited source or one `alef.toml` key restamped every
//! generated file in the tree, and 98.8% of a consumer's regen diff was
//! provenance noise. The whole-tree fingerprint
//! ([`crate::core::hash::compute_inputs_hash`]) is recorded once per crate
//! instead, via `cache::record_inputs_hash`. The hashes in this module are a
//! *separate mechanism* again: they are
//! skip-or-regenerate decisions held in `.alef/`, never written into generated
//! output. Adding an input here changes what is regenerated; it does not change
//! a single byte of any file's embedded hash. ~keep

/// A `.alef/` cache key that is permitted to gate a skip decision.
///
/// Constructible only by this module's `compute_*` functions, each of which folds
/// in the alef build identity. See the module doc for why that is a type
/// invariant rather than a convention. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    value: String,
}

impl CacheKey {
    /// The key's on-disk textual form, for writing to and comparing against `.alef/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// Mint a distinct [`CacheKey`] per `label`, for tests that only need two keys to differ.
///
/// Goes through a real builder rather than exposing a constructor: [`CacheKey`] is unforgeable
/// on purpose, and a back door here — even a `#[cfg(test)]` one — would be the first thing a
/// future cache reached for. ~keep
#[cfg(test)]
pub(crate) fn key_for_test(label: &str) -> CacheKey {
    compute_lang_hash("", label, "")
}

/// Finish a key, folding in the alef build identity that every skip decision depends on.
///
/// The only constructor of [`CacheKey`]. Routing all three key builders through it is what
/// makes "no cache entry outlives the alef build that wrote it" a property of the module
/// rather than of each call site remembering. ~keep
fn finish(mut hasher: blake3::Hasher, alef_version: &str) -> CacheKey {
    hasher.update(binary_identity().as_bytes());
    hasher.update(alef_version.as_bytes());
    CacheKey {
        value: hasher.finalize().to_hex().to_string(),
    }
}

/// The alef version that compiled this binary, baked in at build time.
///
/// This is the load-bearing input that [`binary_identity`] cannot be trusted to
/// supply — see the invalidation note on [`compute_stage_hash`]. ~keep
pub(crate) fn alef_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Return a string representing the running alef binary's identity: mtime_nanos + file size.
/// Used to salt cache keys so that a locally-rebuilt binary always invalidates stale caches.
///
/// Best-effort only. Every step (`current_exe`, `metadata`, `modified`) can fail, and the
/// whole chain collapses to the empty string when it does, contributing nothing to the key
/// while still *looking* like an invalidation input. It catches the case a version number
/// cannot — two different builds of the same version, i.e. a local `cargo install --path .`
/// — so it stays, but it must never be the only thing standing between a new alef release
/// and a stale cache. ~keep
fn binary_identity() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(&p).ok())
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

/// Compute the cache key for an extracted `ApiSurface` (`.alef/<crate>/ir.{json,hash}`).
///
/// `schema_version` is the serialization generation of `ir.json` itself; the remaining
/// components are the extraction inputs. The alef build identity is folded in by [`finish`]
/// because the *extractor* is an input too: `src/extract/` changes between releases, and a
/// surface produced by an older one is not the surface the current binary would produce.
/// Nothing else invalidates this cache on an upgrade — the consumer's own `Cargo.toml`
/// version reaches it only via `crate_version`, which is why an unrelated version bump was
/// what finally dislodged a months-stale surface in the field. ~keep
pub fn compute_ir_key(schema_version: &str, sources_hash: &str, crate_version: &str, config_hash: &str) -> CacheKey {
    compute_ir_key_for_version(alef_version(), schema_version, sources_hash, crate_version, config_hash)
}

/// Compute hash for a language's output (IR + language-specific config + alef build identity).
pub fn compute_lang_hash(ir_json: &str, lang: &str, config_toml: &str) -> CacheKey {
    compute_lang_hash_for_version(alef_version(), ir_json, lang, config_toml)
}

/// Compute hash for a generation stage (stubs, docs, readme, scaffold, e2e).
/// `extra` allows including additional content (e.g., fixture files for e2e).
///
/// The alef build doing the generating is part of the key. A new alef release emits
/// different bytes from byte-identical inputs, so an upgrade must invalidate the stage
/// cache — otherwise `alef generate` reports every stage "up to date (skipping)" and the
/// consumer silently keeps the previous release's generated code until someone happens to
/// pass `--clean`. The compile-time version is what guarantees that; [`binary_identity`] is
/// an additional, failure-prone salt that only narrows same-version rebuilds. ~keep
pub fn compute_stage_hash(ir_json: &str, stage: &str, config_toml: &str, extra: &[u8]) -> CacheKey {
    compute_stage_hash_for_version(alef_version(), ir_json, stage, config_toml, extra)
}

/// [`compute_ir_key`] with the alef version supplied by the caller, so tests can
/// observe the effect of an alef upgrade without rebuilding the binary.
pub(crate) fn compute_ir_key_for_version(
    alef_version: &str,
    schema_version: &str,
    sources_hash: &str,
    crate_version: &str,
    config_hash: &str,
) -> CacheKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(schema_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(sources_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(crate_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(config_hash.as_bytes());
    finish(hasher, alef_version)
}

/// [`compute_lang_hash`] with the alef version supplied by the caller, so tests can
/// observe the effect of an alef upgrade without rebuilding the binary.
fn compute_lang_hash_for_version(alef_version: &str, ir_json: &str, lang: &str, config_toml: &str) -> CacheKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(lang.as_bytes());
    hasher.update(config_toml.as_bytes());
    finish(hasher, alef_version)
}

/// [`compute_stage_hash`] with the alef version supplied by the caller, so tests can
/// observe the effect of an alef upgrade without rebuilding the binary.
fn compute_stage_hash_for_version(
    alef_version: &str,
    ir_json: &str,
    stage: &str,
    config_toml: &str,
    extra: &[u8],
) -> CacheKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(stage.as_bytes());
    hasher.update(config_toml.as_bytes());
    if !extra.is_empty() {
        hasher.update(extra);
    }
    finish(hasher, alef_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IR: &str = r#"{"crate_name":"sample","functions":[]}"#;
    const CONFIG: &str = "[workspace]\nlanguages = [\"node\"]\n";

    /// The regression under test: an alef upgrade must invalidate the stage cache.
    ///
    /// Before the compile-time version became part of the key, the only alef-build input was
    /// `binary_identity()` — a runtime `current_exe()` mtime+size probe that returns the empty
    /// string on any failure. Two different alef releases could therefore produce an identical
    /// stage hash, `is_stage_cached` would answer "hit", and the whole regeneration became a
    /// no-op that reported success. ~keep
    #[test]
    fn stage_hash_changes_when_the_alef_version_changes() {
        let before = compute_stage_hash_for_version("0.62.12", IR, "stubs", CONFIG, &[]);
        let after = compute_stage_hash_for_version("0.64.0", IR, "stubs", CONFIG, &[]);
        assert_ne!(
            before, after,
            "a new alef release must invalidate the stage cache; identical hashes mean \
             `alef generate` skips every stage and silently keeps the old release's output"
        );
    }

    const IR_SCHEMA: &str = "ir-cache-v2";
    const SOURCES_HASH: &str = "sources-abc";
    const CRATE_VERSION: &str = "1.4.0";
    const CONFIG_HASH: &str = "config-abc";

    /// The third cache, and the one that shipped without the salt its two siblings already
    /// had. A newer alef reusing an older alef's `ApiSurface` is worse than reusing its
    /// generated bytes: `#[serde(default)]` on most `ApiSurface` fields means a field the
    /// older extractor never wrote deserializes to its default instead of failing, so the
    /// new binary generates from a surface that is wrong rather than merely old — and
    /// `alef verify`, which reads the same cache, agrees with it. ~keep
    #[test]
    fn ir_key_changes_when_the_alef_version_changes() {
        let before = compute_ir_key_for_version("0.67.2", IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, CONFIG_HASH);
        let after = compute_ir_key_for_version("0.67.5", IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, CONFIG_HASH);
        assert_ne!(
            before, after,
            "a new alef release must invalidate the IR cache; identical keys mean the new \
             binary extracts nothing and generates from the previous release's ApiSurface"
        );
    }

    /// Each extraction input must still move the key on its own — otherwise the version salt
    /// would be masking a key that had stopped distinguishing anything else. ~keep
    #[test]
    fn ir_key_still_separates_every_extraction_input_within_one_version() {
        let version = "0.67.5";
        let base = compute_ir_key_for_version(version, IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, CONFIG_HASH);
        assert_ne!(
            base,
            compute_ir_key_for_version(version, "ir-cache-v3", SOURCES_HASH, CRATE_VERSION, CONFIG_HASH)
        );
        assert_ne!(
            base,
            compute_ir_key_for_version(version, IR_SCHEMA, "sources-xyz", CRATE_VERSION, CONFIG_HASH)
        );
        assert_ne!(
            base,
            compute_ir_key_for_version(version, IR_SCHEMA, SOURCES_HASH, "1.5.0", CONFIG_HASH)
        );
        assert_ne!(
            base,
            compute_ir_key_for_version(version, IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, "config-xyz")
        );
        assert_eq!(
            base,
            compute_ir_key_for_version(version, IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, CONFIG_HASH),
            "the IR key must be deterministic within one alef build"
        );
    }

    /// The public entry point must route through the compiled-in version, not merely expose a
    /// version-aware helper that `pipeline::extract` does not call. ~keep
    #[test]
    fn compute_ir_key_uses_the_compiled_in_alef_version() {
        assert_eq!(
            compute_ir_key(IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, CONFIG_HASH),
            compute_ir_key_for_version(
                env!("CARGO_PKG_VERSION"),
                IR_SCHEMA,
                SOURCES_HASH,
                CRATE_VERSION,
                CONFIG_HASH
            )
        );
    }

    /// Same defect, same fix, on the sibling cache. `[node] up to date (skipping)` — the
    /// symptom actually reported from a consumer repo — is the *language* cache, not the
    /// stage cache. ~keep
    #[test]
    fn lang_hash_changes_when_the_alef_version_changes() {
        let before = compute_lang_hash_for_version("0.62.12", IR, "node", CONFIG);
        let after = compute_lang_hash_for_version("0.64.0", IR, "node", CONFIG);
        assert_ne!(
            before, after,
            "a new alef release must invalidate the per-language cache"
        );
    }

    /// The regression this closes: every test above proves only that two [`CacheKey`]s differ in
    /// isolation. None of them prove that difference reaches the read path a real `alef generate`
    /// run actually takes -- `cli::cache::is_ir_cached`/`is_lang_cached`/`is_stage_cached`, the
    /// functions `pipeline::extract`, `pipeline::generate::generation`, and every stage call site
    /// (`bin_cli::core_commands`, `bin_cli::all_commands::e2e_stage`, ...) call to decide
    /// skip-vs-regenerate. This writes a real on-disk cache entry keyed to alef "0.62.12", then
    /// queries it through those same public functions keyed to alef "0.64.0" for byte-identical
    /// inputs, and requires a miss on all three caches -- proving an alef upgrade actually forces
    /// regeneration rather than merely producing a `CacheKey` that nothing reads differently. ~keep
    #[test]
    fn upgrading_the_alef_version_turns_a_real_cache_hit_into_a_miss() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(tmp.path());

        assert_ir_cache_miss_after_version_bump();
        assert_lang_cache_miss_after_version_bump(tmp.path());
        assert_stage_cache_miss_after_version_bump(tmp.path());
    }

    fn assert_ir_cache_miss_after_version_bump() {
        let api = crate::core::ir::ApiSurface {
            crate_name: "sample-crate".to_string(),
            ..Default::default()
        };
        let old_key = compute_ir_key_for_version("0.62.12", IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, CONFIG_HASH);
        crate::cli::cache::write_ir_cache("sample-crate", &api, &old_key).expect("write ir cache");

        assert!(
            crate::cli::cache::is_ir_cached("sample-crate", &old_key),
            "querying with the version that wrote the cache must still be a hit"
        );

        let new_key = compute_ir_key_for_version("0.64.0", IR_SCHEMA, SOURCES_HASH, CRATE_VERSION, CONFIG_HASH);
        assert!(
            !crate::cli::cache::is_ir_cached("sample-crate", &new_key),
            "a newer alef querying the same on-disk IR cache for byte-identical extraction \
             inputs must be a miss, or it replays a previous release's ApiSurface verbatim"
        );
    }

    fn assert_lang_cache_miss_after_version_bump(root: &std::path::Path) {
        let generated = root.join("bindings.py");
        std::fs::write(&generated, "# generated\n").expect("write generated output");

        let old_key = compute_lang_hash_for_version("0.62.12", IR, "python", CONFIG);
        crate::cli::cache::write_lang_hash("sample-crate", "python", &old_key, &[generated])
            .expect("write lang hash and manifest");

        assert!(
            crate::cli::cache::is_lang_cached("sample-crate", "python", &old_key),
            "querying with the version that wrote the cache must still be a hit"
        );

        let new_key = compute_lang_hash_for_version("0.64.0", IR, "python", CONFIG);
        assert!(
            !crate::cli::cache::is_lang_cached("sample-crate", "python", &new_key),
            "a newer alef querying the same on-disk language cache for byte-identical inputs \
             must be a miss, or `alef generate` reports the language up to date and skips it"
        );
    }

    fn assert_stage_cache_miss_after_version_bump(root: &std::path::Path) {
        let output = root.join("README.md");
        std::fs::write(&output, "generated readme\n").expect("write stage output");

        let old_key = compute_stage_hash_for_version("0.62.12", IR, "readme", CONFIG, &[]);
        crate::cli::cache::write_stage_hash("sample-crate", "readme", old_key.as_str(), &[output])
            .expect("write stage hash and manifest");

        assert!(
            crate::cli::cache::is_stage_cached("sample-crate", "readme", &old_key),
            "querying with the version that wrote the cache must still be a hit"
        );

        let new_key = compute_stage_hash_for_version("0.64.0", IR, "readme", CONFIG, &[]);
        assert!(
            !crate::cli::cache::is_stage_cached("sample-crate", "readme", &new_key),
            "a newer alef querying the same on-disk stage cache must be a miss, or `alef generate` \
             reports the stage up to date and skips it"
        );
    }

    #[test]
    fn stage_hash_still_separates_stages_and_inputs_within_one_version() {
        let version = "0.64.0";
        let base = compute_stage_hash_for_version(version, IR, "stubs", CONFIG, &[]);
        assert_ne!(base, compute_stage_hash_for_version(version, IR, "docs", CONFIG, &[]));
        assert_ne!(
            base,
            compute_stage_hash_for_version(version, r#"{"crate_name":"other"}"#, "stubs", CONFIG, &[])
        );
        assert_ne!(
            base,
            compute_stage_hash_for_version(version, IR, "stubs", "[workspace]\n", &[])
        );
        assert_ne!(
            base,
            compute_stage_hash_for_version(version, IR, "stubs", CONFIG, b"fixture")
        );
    }

    #[test]
    fn hashes_are_deterministic_for_one_version() {
        assert_eq!(
            compute_stage_hash_for_version("0.64.0", IR, "stubs", CONFIG, &[]),
            compute_stage_hash_for_version("0.64.0", IR, "stubs", CONFIG, &[])
        );
        assert_eq!(
            compute_lang_hash_for_version("0.64.0", IR, "node", CONFIG),
            compute_lang_hash_for_version("0.64.0", IR, "node", CONFIG)
        );
    }

    /// The public entry points must route through the compile-time version, not merely
    /// expose a version-aware helper that nothing calls. ~keep
    #[test]
    fn public_entry_points_use_the_compiled_in_alef_version() {
        assert_eq!(
            compute_stage_hash(IR, "stubs", CONFIG, &[]),
            compute_stage_hash_for_version(env!("CARGO_PKG_VERSION"), IR, "stubs", CONFIG, &[])
        );
        assert_eq!(
            compute_lang_hash(IR, "node", CONFIG),
            compute_lang_hash_for_version(env!("CARGO_PKG_VERSION"), IR, "node", CONFIG)
        );
    }

    /// Guards the constraint that makes this fix safe to ship: the embedded `alef:hash:`
    /// value is a different mechanism and must not move. If folding the alef version into
    /// the cache keys had leaked into `compute_inputs_hash`, every generated file in every
    /// consumer would restamp on every alef release — the exact cost the stripped
    /// `alef_version` pin exists to avoid. These are frozen expected values, so they fail on
    /// any drift rather than re-deriving whatever the code currently does.
    /// Regenerated 2026-08-27 for the CODEGEN_FORMAT_VERSION 2 -> 3 bump, which
    /// `compute_inputs_hash` folds in -- that is the ONE legitimate reason these move.
    /// Any other drift is the bug this test exists to catch. ~keep
    #[test]
    fn embedded_inputs_hash_is_unaffected_by_cache_key_identity() {
        let cases: [(&str, &[u8], &str); 3] = [
            (
                "sources-abc",
                b"[workspace]\nlanguages = [\"node\"]\n",
                "5bec01d0c5156f7339c8e1dd14b9245fbf791933156ec252cab9bce3ae8e0e39",
            ),
            (
                "sources-abc",
                b"",
                "6236d147c8bf5653a1aef09d70abdd3ecb1f0cb2e27056b0f03d277c3310bc00",
            ),
            (
                "",
                b"[workspace]\n",
                "8ada6034d43c7dbb07d46e8194b70cc5cc9ad4c3db53342d1bc80d76d7487f2f",
            ),
        ];
        for (sources_hash, toml_bytes, expected) in cases {
            assert_eq!(
                crate::core::hash::compute_inputs_hash(sources_hash, toml_bytes),
                expected,
                "compute_inputs_hash drifted for sources_hash={sources_hash:?}; the embedded \
                 alef:hash: value must not change when cache-key inputs change"
            );
        }
    }

    /// The `alef_version` pin is stripped from the embedded inputs hash but is *not* stripped
    /// from the raw `alef.toml` text the cache keys hash. Both halves of that asymmetry are
    /// load-bearing and easy to break in opposite directions. ~keep
    #[test]
    fn alef_version_pin_moves_cache_keys_but_not_the_embedded_inputs_hash() {
        let pinned_old = b"[workspace]\nlanguages = [\"node\"]\nalef_version = \"0.62.12\"\n";
        let pinned_new = b"[workspace]\nlanguages = [\"node\"]\nalef_version = \"0.64.0\"\n";

        assert_eq!(
            crate::core::hash::compute_inputs_hash("sources-abc", pinned_old),
            crate::core::hash::compute_inputs_hash("sources-abc", pinned_new),
            "bumping the alef_version pin must not restamp generated files"
        );

        let version = "0.64.0";
        assert_ne!(
            compute_stage_hash_for_version(version, IR, "stubs", &String::from_utf8_lossy(pinned_old), &[]),
            compute_stage_hash_for_version(version, IR, "stubs", &String::from_utf8_lossy(pinned_new), &[]),
            "the cache keys hash raw alef.toml text, so a pin bump must still invalidate them"
        );
    }
}
