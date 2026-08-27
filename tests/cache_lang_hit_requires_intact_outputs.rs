//! A per-language cache hit must mean the manifested outputs are still what alef wrote.
//!
//! A consumer repo falsified the warm path directly: it appended a line to a generated file, ran
//! `alef generate` with `.alef/` intact, and got `Generated 0 files` with the appended line still
//! in place. Deleting `.alef/` made the same command restore the file. The cached path could not
//! tell a correct tree from a corrupted one — not stale output, but a check that examined
//! nothing and reported success.
//!
//! The cause is here: `is_lang_cached`'s manifest check tested each recorded output for
//! *existence*. A hand-edited file still exists, so the language stayed a hit and was dropped
//! from the generation set entirely, which is also why the content comparison downstream
//! (`generated_files_match_disk`) never ran — there was nothing left to compare.
//!
//! Asserted on the predicate rather than through the CLI deliberately. The end-to-end
//! reproduction is real (it was run by hand, and the fix was confirmed against it the same way),
//! but a fixture-driven version of it is not a sound regression test: a *cached* run writes an
//! empty language manifest, an empty manifest is already a miss, and whether the second run of a
//! small fixture lands on the hit path at all varied between otherwise identical invocations.
//! A test whose subject moves is worse than no test, and this file's subject does not move. ~keep

use std::fs;
use std::path::{Path, PathBuf};

use alef::cli::cache;
use alef::core::hash::{CommentStyle, compute_file_hash, header, inject_hash_line};

const CRATE_NAME: &str = "sample-crate";
const LANG: &str = "go";
const BODY: &str = "package bindings\n\nfunc RecordValue(v string) string { return v }\n";

/// Write a file stamped exactly the way generation stamps one: alef header, then the
/// `alef:hash:` line carrying `compute_file_hash(content)`.
fn write_stamped(path: &Path) {
    let content = format!("{}{BODY}", header(CommentStyle::DoubleSlash));
    let stamped = inject_hash_line(&content, &compute_file_hash(&content));
    fs::write(path, stamped).expect("write stamped generated file");
}

fn record_manifest(outputs: &[PathBuf]) -> cache::CacheKey {
    let key = cache::compute_lang_hash("ir", LANG, "config");
    cache::write_lang_hash(CRATE_NAME, LANG, &key, outputs).expect("record language hash and manifest");
    key
}

/// One test per file: `is_lang_cached` resolves `.alef/` against the process working directory,
/// and `set_current_dir` is process-global, so a second test in this binary could observe this
/// one's directory. The crate's own `CwdGuard` is `pub(crate)` and unavailable here.
#[test]
fn a_stamped_output_edited_after_generation_turns_the_cache_hit_into_a_miss() {
    let fixture = tempfile::tempdir().expect("create fixture directory");
    std::env::set_current_dir(fixture.path()).expect("enter fixture directory");
    let root = std::env::current_dir().expect("read fixture directory");

    let stamped = root.join("binding.go");
    let unstamped = root.join("go.mod");
    write_stamped(&stamped);
    fs::write(&unstamped, "module example.com/bindings\n").expect("write unstamped output");

    let key = record_manifest(&[stamped.clone(), unstamped.clone()]);
    assert!(
        cache::is_lang_cached(CRATE_NAME, LANG, &key),
        "an untouched tree whose manifested outputs all agree with their stamps must be a hit; \
         without this the miss assertions below would pass for the wrong reason"
    );

    let pristine = fs::read_to_string(&stamped).expect("read stamped output");
    fs::write(&stamped, format!("{pristine}// a hand edit\n")).expect("append to the stamped output");
    assert!(
        !cache::is_lang_cached(CRATE_NAME, LANG, &key),
        "a manifested output edited after generation must be a cache MISS; as a hit, `alef \
         generate` drops the language and answers `Generated 0 files` without ever reading the \
         file it just vouched for"
    );

    fs::write(&stamped, &pristine).expect("restore the stamped output");
    assert!(
        cache::is_lang_cached(CRATE_NAME, LANG, &key),
        "restoring the file byte-for-byte must restore the hit; the check must key on content, \
         not on the file having been touched"
    );

    // An unstamped output — `generated_header: false`, a create-once seed — carries nothing to
    // compare against, so it keeps the existence-only rule rather than forcing a permanent miss.
    fs::write(&unstamped, "module example.com/renamed\n").expect("edit the unstamped output");
    assert!(
        cache::is_lang_cached(CRATE_NAME, LANG, &key),
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
        !cache::is_lang_cached(CRATE_NAME, LANG, &key),
        "a missing manifested output must still be a miss"
    );
}
