//! Cache Directory Tagging (<https://bford.info/cachedir/>) for every directory alef uses purely
//! as regenerable cache or scratch space.
//!
//! A directory tagged this way is a stable, tool-agnostic declaration that its contents can be
//! deleted and rebuilt for free. Backup and sync tools that honour the spec (`tar
//! --exclude-caching`, `rsync --exclude-tag`, Borg, restic, several `du` variants, ...) skip it
//! automatically, the same way cargo's `target/` and `~/.cargo/registry`, uv's cache, and
//! Gradle's `~/.gradle/caches` already do. It also lets external tooling recognise an alef cache
//! directory without depending on alef's private file names inside it (`zig-hashes.json`,
//! `ir.json`, ...), which are free to change in any refactor.
//!
//! [`ensure_cache_dir`] is the single call every cache-directory creation site in this crate goes
//! through, so the tag is written -- or correctly left alone -- in exactly one place. Do not
//! duplicate this logic at a call site; add a call to [`ensure_cache_dir`] instead. It must never
//! be pointed at a directory that holds committed, hand-editable, or otherwise non-regenerable
//! content -- see the callers in `cli::cache` that create `base_dir` itself for the committed
//! `.alef-ownership.toml` / `.alef-toml-merge-provenance.toml` records, which deliberately do
//! *not* call this function. ~keep

use std::io;
use std::path::Path;

/// The exact CACHEDIR.TAG signature, byte-for-byte per the spec. A conforming reader requires
/// this to be the tag file's first line and nothing else on that line -- not a prefix, not a
/// substring elsewhere in the file. ~keep
const SIGNATURE_LINE: &str = "Signature: 8985a1d0364e3d1e-cache-directory-tag";

const TAG_FILE_NAME: &str = "CACHEDIR.TAG";

/// Body written for a freshly created tag. Everything after the signature line is free-form
/// commentary a backup tool ignores; naming alef and linking the spec here is convention, not
/// contract -- see [`SIGNATURE_LINE`] for the one line that is.
fn tag_body() -> String {
    format!(
        "{SIGNATURE_LINE}\n\
         # This directory contains a cache created by alef. Deleting it only costs a rebuild.\n\
         # For information about cache directory tags see https://bford.info/cachedir/\n"
    )
}

/// Create `dir` (and any missing parents) and make sure it carries a valid `CACHEDIR.TAG`.
///
/// Directory creation failures are returned to the caller -- every existing call site already
/// treats "the cache directory could not be created" as fatal to whatever write was about to
/// follow, and that behaviour is unchanged here. Tag failures are never returned: per this
/// repo's tracing level contract a cache directory that works but is untagged is a
/// degraded-but-continuing condition (`WARN`), not a build failure, so a permissions error or an
/// unrecognised file at the tag path is logged and swallowed rather than propagated. See
/// [`ensure_tag`] for the idempotence and invalid-tag rules.
pub fn ensure_cache_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    ensure_tag(dir);
    Ok(())
}

/// Idempotently ensure `dir` -- which must already exist -- carries a valid `CACHEDIR.TAG`,
/// without creating `dir` itself. Split out purely so [`ensure_cache_dir`]'s own doc can stay
/// focused on the create-then-tag contract every caller actually depends on.
fn ensure_tag(dir: &Path) {
    let tag_path = dir.join(TAG_FILE_NAME);
    match std::fs::read(&tag_path) {
        Ok(existing) => {
            // Idempotence: a valid tag is never rewritten, so an operator's edited comment body
            // and the file's mtime both survive every subsequent run.
            if has_valid_signature(&existing) {
                return;
            }
            // Invalid tag: something occupies this reserved name whose first line is not the
            // signature -- foreign content, or a truncated write from a crash. Never overwrite
            // content this function did not itself write; the safe failure here is "this run's
            // cache directory stays untagged," not "silently destroy a file we don't recognise."
            tracing::warn!(
                path = %tag_path.display(),
                "a file named {TAG_FILE_NAME} exists here but its first line is not the \
                 cache-directory-tag signature; leaving it untouched instead of overwriting \
                 content alef did not write -- this directory will not be recognised as a cache \
                 by tools that honour the tag until it is repaired or removed by hand"
            );
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Err(write_error) = std::fs::write(&tag_path, tag_body()) {
                tracing::warn!(
                    path = %tag_path.display(),
                    error = %write_error,
                    "could not write {TAG_FILE_NAME}; the cache directory itself still works, \
                     but backup/sync tools that honour the tag will not skip it"
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                path = %tag_path.display(),
                error = %error,
                "could not read the existing {TAG_FILE_NAME} to check it; leaving this cache \
                 directory untagged for this run rather than guessing whether it is safe to write"
            );
        }
    }
}

/// Whether `content`'s first line -- up to but not including the first `\n` -- is byte-identical
/// to [`SIGNATURE_LINE`]. Deliberately not a substring/`contains` check: a signature on line two
/// is not a valid tag per spec, and a reader that accepted it would tag directories a real
/// CACHEDIR.TAG-aware tool does not.
fn has_valid_signature(content: &[u8]) -> bool {
    let first_line = content.split(|&byte| byte == b'\n').next().unwrap_or(content);
    first_line == SIGNATURE_LINE.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creating_a_cache_dir_writes_a_tag_with_the_exact_signature_as_its_first_line() {
        let root = tempfile::tempdir().expect("temp dir");
        let cache_dir = root.path().join("cache");

        ensure_cache_dir(&cache_dir).expect("ensure_cache_dir must succeed");

        assert!(cache_dir.is_dir(), "the cache directory itself must have been created");
        let tag_content = std::fs::read_to_string(cache_dir.join(TAG_FILE_NAME)).expect("read CACHEDIR.TAG");
        let first_line = tag_content.lines().next().expect("tag file must have a first line");
        assert_eq!(
            first_line, SIGNATURE_LINE,
            "the first line must equal the signature exactly, not merely contain it"
        );
    }

    #[test]
    fn a_second_run_does_not_rewrite_an_existing_valid_tag() {
        let root = tempfile::tempdir().expect("temp dir");
        let cache_dir = root.path().join("cache");
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");
        let tag_path = cache_dir.join(TAG_FILE_NAME);
        let custom_body = format!("{SIGNATURE_LINE}\n# a human-edited comment that must survive\n");
        std::fs::write(&tag_path, &custom_body).expect("plant an existing valid tag");
        let mtime_before = std::fs::metadata(&tag_path).expect("stat before").modified().expect("mtime");

        // Guarantee the filesystem mtime clock has a chance to move, so an accidental rewrite
        // would be observable even on filesystems with coarse mtime resolution.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        ensure_cache_dir(&cache_dir).expect("second call must succeed");

        let content_after = std::fs::read_to_string(&tag_path).expect("read tag after second run");
        let mtime_after = std::fs::metadata(&tag_path).expect("stat after").modified().expect("mtime");
        assert_eq!(
            content_after, custom_body,
            "an existing valid tag's content, including a user-added comment, must survive byte-for-byte"
        );
        assert_eq!(
            mtime_before, mtime_after,
            "an existing valid tag must not be rewritten -- its mtime must not move"
        );
    }

    #[test]
    fn an_invalid_existing_tag_file_is_left_untouched_rather_than_clobbered() {
        let root = tempfile::tempdir().expect("temp dir");
        let cache_dir = root.path().join("cache");
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");
        let tag_path = cache_dir.join(TAG_FILE_NAME);
        let foreign_content = "not a cache directory tag\njust some other file\n";
        std::fs::write(&tag_path, foreign_content).expect("plant a foreign, non-signature file");

        ensure_cache_dir(&cache_dir).expect("must not error even when the tag path is occupied");

        let content_after = std::fs::read_to_string(&tag_path).expect("read after ensure_cache_dir");
        assert_eq!(
            content_after, foreign_content,
            "a file at CACHEDIR.TAG whose first line is not the signature must never be overwritten"
        );
    }

    #[test]
    fn a_directory_that_cannot_be_tagged_is_still_created_and_usable() {
        let root = tempfile::tempdir().expect("temp dir");
        let cache_dir = root.path().join("cache");
        std::fs::create_dir_all(&cache_dir).expect("create cache dir");
        // Occupy the tag's reserved name with a directory instead of a file, so both the read
        // (to check for an existing valid tag) and any write attempt fail without relying on
        // platform-specific permission bits.
        std::fs::create_dir_all(cache_dir.join(TAG_FILE_NAME)).expect("occupy tag path with a directory");

        let result = ensure_cache_dir(&cache_dir);

        assert!(
            result.is_ok(),
            "a cache directory that cannot be tagged must still succeed, not fail the caller: {result:?}"
        );
        assert!(
            cache_dir.is_dir(),
            "the cache directory itself must remain usable regardless of the tag outcome"
        );
    }

    #[test]
    fn has_valid_signature_rejects_the_signature_on_a_later_line() {
        // Proves this cannot be satisfied by a `contains`-style check: the signature is present
        // in the bytes, but not as the first line, so it must be rejected.
        let content = format!("not the first line\n{SIGNATURE_LINE}\n");
        assert!(!has_valid_signature(content.as_bytes()));
    }

    #[test]
    fn has_valid_signature_accepts_exactly_the_signature_with_trailing_content() {
        let content = format!("{SIGNATURE_LINE}\n# trailing comment\n");
        assert!(has_valid_signature(content.as_bytes()));
    }
}
