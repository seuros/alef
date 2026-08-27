//! A per-stage cache hit must mean the manifested outputs are still what alef wrote.
//!
//! `faa3f4b83` fixed this for the per-language cache (`is_lang_cached`): a hit used to mean
//! only "every recorded output still exists", so a consumer's hand-edit to a generated binding
//! file survived a cache hit undetected -- `alef generate` answered `Generated 0 files` with the
//! edit still in place. `is_stage_cached` -- the sibling predicate for the e2e, scaffold,
//! readme, and docs stages -- had a correctly salted key (it already routed through
//! `cache_identity::CacheKey`) but kept the exact same existence-only manifest check the
//! language cache was fixed away from. A consumer who edits a generated e2e test file, README,
//! or docs page still got a silent skip: the file exists, so the stage is dropped from the work
//! set before anything compares its `alef:hash:` line.
//!
//! Mirrors `tests/cache_lang_hit_requires_intact_outputs.rs` exactly, one predicate over.
//! Asserted on the predicate rather than through the CLI for the same reason that file gives:
//! a *cached* stage run's manifest bookkeeping is wiring spread across several `bin_cli`
//! call sites, and whether a small fixture's second run lands on the hit path at all is not
//! this predicate's own concern to prove.

use std::fs;
use std::path::{Path, PathBuf};

use alef::cli::cache;
use alef::core::hash::{CommentStyle, compute_file_hash, header, inject_hash_line};

const CRATE_NAME: &str = "sample-crate";
const STAGE: &str = "e2e";
const BODY: &str = "def test_record_value():\n    assert True\n";

/// Write a file stamped exactly the way generation stamps one: alef header, then the
/// `alef:hash:` line carrying `compute_file_hash(content)`.
fn write_stamped(path: &Path) {
    let content = format!("{}{BODY}", header(CommentStyle::Hash));
    let stamped = inject_hash_line(&content, &compute_file_hash(&content));
    fs::write(path, stamped).expect("write stamped generated file");
}

fn record_manifest(outputs: &[PathBuf]) -> cache::CacheKey {
    let key = cache::compute_stage_hash("ir", STAGE, "config", &[]);
    cache::write_stage_hash(CRATE_NAME, STAGE, key.as_str(), outputs).expect("record stage hash and manifest");
    key
}

/// One test per file: `is_stage_cached` resolves `.alef/` against the process working directory,
/// and `set_current_dir` is process-global, so a second test in this binary could observe this
/// one's directory. The crate's own `CwdGuard` is `pub(crate)` and unavailable here.
#[test]
fn a_stamped_stage_output_edited_after_generation_turns_the_cache_hit_into_a_miss() {
    let fixture = tempfile::tempdir().expect("create fixture directory");
    std::env::set_current_dir(fixture.path()).expect("enter fixture directory");
    let root = std::env::current_dir().expect("read fixture directory");

    let stamped = root.join("test_smoke.py");
    let unstamped = root.join("conftest.py");
    write_stamped(&stamped);
    fs::write(&unstamped, "# hand-grown pytest fixtures\n").expect("write unstamped output");

    let key = record_manifest(&[stamped.clone(), unstamped.clone()]);
    assert!(
        cache::is_stage_cached(CRATE_NAME, STAGE, &key),
        "an untouched tree whose manifested outputs all agree with their stamps must be a hit; \
         without this the miss assertions below would pass for the wrong reason"
    );

    let pristine = fs::read_to_string(&stamped).expect("read stamped output");
    fs::write(&stamped, format!("{pristine}# a hand edit\n")).expect("append to the stamped output");
    assert!(
        !cache::is_stage_cached(CRATE_NAME, STAGE, &key),
        "a manifested stage output edited after generation must be a cache MISS; as a hit, the \
         stage is dropped from the work set before anything reads the file it just vouched for"
    );

    fs::write(&stamped, &pristine).expect("restore the stamped output");
    assert!(
        cache::is_stage_cached(CRATE_NAME, STAGE, &key),
        "restoring the file byte-for-byte must restore the hit; the check must key on content, \
         not on the file having been touched"
    );

    // An unstamped output -- `generated_header: false`, a create-once seed -- carries nothing to
    // compare against, so it keeps the existence-only rule rather than forcing a permanent miss.
    fs::write(&unstamped, "# a different hand-grown fixture set\n").expect("edit the unstamped output");
    assert!(
        cache::is_stage_cached(CRATE_NAME, STAGE, &key),
        "an output with no alef:hash: stamp has no stamp to disagree with and must not be \
         treated as tampering"
    );

    // The per-file `alef:hash:` is computed over the file's own content alone; folding a
    // whole-tree fingerprint into it is exactly the churn this recipe was split to stop (one
    // edited source restamped 3,436 generated files). This used to be asserted here by passing a
    // *different* inputs hash and demanding a hit. The predicate no longer accepts one at all, so
    // the property is now enforced by the signature rather than by this test -- stale-tree
    // detection lives in the recorded generation fingerprint (`cache::stale_crate_names`,
    // consumed by `alef verify`), never in an individual file's stamp. ~keep

    fs::remove_file(&stamped).expect("delete a manifested output");
    assert!(
        !cache::is_stage_cached(CRATE_NAME, STAGE, &key),
        "a missing manifested output must still be a miss"
    );
}
