//! Cache-key identity for the language and stage generation caches.
//!
//! Split out of [`crate::cli::cache`] so the "what makes a cache entry stale?"
//! question lives in one bounded place. Two hashes are computed here:
//! [`compute_lang_hash`] (per target language, consumed by
//! `cache::is_lang_cached`) and [`compute_stage_hash`] (per generation stage —
//! stubs, docs, readme, scaffold, e2e — consumed by `cache::is_stage_cached`).
//!
//! # These are NOT the embedded `alef:hash:` inputs hash
//!
//! Generated files carry an `alef:hash:` value produced by
//! [`crate::core::hash::compute_inputs_hash`], which is a deliberately narrow
//! function of the Rust sources plus a normalized `alef.toml` (with the
//! `[workspace] alef_version` pin stripped). That value must stay stable across
//! alef releases, or every consumer restamps every generated file on every
//! upgrade. The hashes in this module are a *separate mechanism*: they are
//! skip-or-regenerate decisions held in `.alef/`, never written into generated
//! output. Adding an input here changes what is regenerated; it does not change
//! a single byte of any file's embedded hash. ~keep

/// The alef version that compiled this binary, baked in at build time.
///
/// This is the load-bearing input that [`binary_identity`] cannot be trusted to
/// supply — see the invalidation note on [`compute_stage_hash`]. ~keep
fn alef_version() -> &'static str {
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

/// Compute hash for a language's output (IR + language-specific config + alef build identity).
pub fn compute_lang_hash(ir_json: &str, lang: &str, config_toml: &str) -> String {
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
pub fn compute_stage_hash(ir_json: &str, stage: &str, config_toml: &str, extra: &[u8]) -> String {
    compute_stage_hash_for_version(alef_version(), ir_json, stage, config_toml, extra)
}

/// [`compute_lang_hash`] with the alef version supplied by the caller, so tests can
/// observe the effect of an alef upgrade without rebuilding the binary.
fn compute_lang_hash_for_version(alef_version: &str, ir_json: &str, lang: &str, config_toml: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(lang.as_bytes());
    hasher.update(config_toml.as_bytes());
    hasher.update(binary_identity().as_bytes());
    hasher.update(alef_version.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// [`compute_stage_hash`] with the alef version supplied by the caller, so tests can
/// observe the effect of an alef upgrade without rebuilding the binary.
fn compute_stage_hash_for_version(
    alef_version: &str,
    ir_json: &str,
    stage: &str,
    config_toml: &str,
    extra: &[u8],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ir_json.as_bytes());
    hasher.update(stage.as_bytes());
    hasher.update(config_toml.as_bytes());
    if !extra.is_empty() {
        hasher.update(extra);
    }
    hasher.update(binary_identity().as_bytes());
    hasher.update(alef_version.as_bytes());
    hasher.finalize().to_hex().to_string()
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
    /// any drift rather than re-deriving whatever the code currently does. ~keep
    #[test]
    fn embedded_inputs_hash_is_unaffected_by_cache_key_identity() {
        let cases: [(&str, &[u8], &str); 3] = [
            (
                "sources-abc",
                b"[workspace]\nlanguages = [\"node\"]\n",
                "b87e401397681a7347a51d94496237d2ea3cb721f42e5b136a98e86f2c1f8ecb",
            ),
            (
                "sources-abc",
                b"",
                "e2404a2956e25be55cd89779670638ce19288a7ae787a693cc97e984fb1d4de9",
            ),
            (
                "",
                b"[workspace]\n",
                "911e97c6dce240140b3c74c57ed23a41535202f33c586a6f6101d3b7f8b4623d",
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
