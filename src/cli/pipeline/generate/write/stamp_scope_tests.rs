//! Regression coverage for the stamp-scope/verify-scope agreement invariant.
//!
//! `alef verify` decides which files it holds to an `alef:hash:` stamp by reading the bytes
//! on disk; the stamping pass decides which files it re-stamps from the in-memory
//! [`crate::core::backend::GeneratedFile`]. Whenever those two answers differ, the file in
//! the gap is reported stale by every future run and no run can clear it. Both halves of
//! every assertion below therefore run the real production functions --
//! [`super::stampable_output_paths`] / [`super::finalize_hashes`] on the writer side and
//! [`crate::bin_cli::helpers::verify_walk`] on the reader side -- never a local restatement
//! of either, because a reimplementation would agree with itself precisely while the two
//! real call paths disagreed. ~keep

use super::{ensure_generated_header, finalize_hashes, stampable_output_paths};
use crate::bin_cli::helpers::verify_walk;
use crate::core::backend::GeneratedFile;
use crate::core::hash;
use std::path::{Path, PathBuf};

/// Two opaque stand-ins for a crate's `sources_hash`. Only their being *different* matters:
/// together they model any input change that moves `compute_inputs_hash` -- a source edit,
/// an `alef.toml` change, or a bump of `CODEGEN_FORMAT_VERSION`, which is the recorded
/// revision of the stamp computation itself.
const SOURCES_HASH_BEFORE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SOURCES_HASH_AFTER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

const ALEF_TOML: &[u8] = b"[workspace]\nlanguages = [\"go\"]\n";
/// A second, unrelated `alef.toml` -- together with [`ALEF_TOML`], models a config edit that
/// changes `compute_inputs_hash` without changing any source file.
const ALEF_TOML_ALT: &[u8] = b"[workspace]\nlanguages = [\"go\", \"python\"]\n";

/// A create-once scaffold seed's emitted body, exactly as the scaffold generator produces it:
/// no alef header, no marker. `packages/go/go.mod` is the shape being modelled; the name is
/// a neutral fixture module.
const SEED_BODY: &str = "module example.invalid/fixture\n\ngo 1.26\n";

fn join(root: &Path, relative: &str) -> PathBuf {
    relative.split('/').fold(root.to_path_buf(), |acc, part| acc.join(part))
}

/// Put `body` on disk headered and stamped the way an earlier alef run left it: marker
/// present, `alef:hash:` computed from the file's own content alone. Returns the absolute
/// path. `sources_hash` is accepted only so callers below can express "this fixture predates
/// a later run under different generation inputs" -- it plays no role in the stamp itself,
/// which never depends on it. See `core::hash`'s module doc.
fn seed_stamped_by_an_earlier_run(root: &Path, relative: &str, body: &str, _sources_hash: &str) -> PathBuf {
    let path = join(root, relative);
    std::fs::create_dir_all(path.parent().expect("fixture path has a parent")).expect("create parent dirs");
    let headered = ensure_generated_header(&path, body);
    let stamped = hash::inject_hash_line(&headered, &hash::compute_file_hash(&headered));
    std::fs::write(&path, &stamped).expect("write seeded file");
    path
}

fn stale_paths(root: &Path) -> Vec<String> {
    verify_walk(root)
        .expect("verify walk")
        .into_iter()
        .map(|mismatch| mismatch.path)
        .collect()
}

/// The load-bearing anti-churn case, replacing this module's previous test of the opposite
/// (and, per `core::hash`'s module doc, actively harmful) claim: that a stamp "has to be
/// refreshed when the stamp computation moves, not only when the bytes do". That was exactly
/// the bug -- a stamp folding `inputs_hash` in meant any unrelated source or `alef.toml`
/// change restamped every generated file, even ones whose own emitted bytes never moved.
///
/// Table-driven over the two independent halves of `inputs_hash`
/// (`compute_inputs_hash(sources_hash, alef_toml_bytes)`): a source-file edit changes
/// `sources_hash` without touching `alef.toml`, and an `alef.toml` edit changes the config
/// bytes without touching any source file. Both must leave an untouched file's stamp alone.
///
/// Each case emits **byte-identical content** under **different** generation inputs. A
/// create-once seed is used specifically because it is the case this module's sibling tests
/// already prove reaches `finalize_hashes` on every run regardless of scope, so this
/// exercises the real write path, not a restatement of it. ~keep
#[test]
fn create_once_seed_marked_on_disk_is_not_restamped_when_only_generation_inputs_move() {
    struct Case {
        name: &'static str,
        sources_hash_after: &'static str,
        alef_toml_after: &'static [u8],
    }

    let cases = [
        Case {
            name: "an unrelated source file changes (sources_hash moves, alef.toml does not)",
            sources_hash_after: SOURCES_HASH_AFTER,
            alef_toml_after: ALEF_TOML,
        },
        Case {
            name: "an unrelated alef.toml key changes (alef.toml moves, sources_hash does not)",
            sources_hash_after: SOURCES_HASH_BEFORE,
            alef_toml_after: ALEF_TOML_ALT,
        },
    ];

    for case in cases {
        let root = tempfile::tempdir().expect("tempdir");
        let path = seed_stamped_by_an_earlier_run(root.path(), "packages/go/go.mod", SEED_BODY, SOURCES_HASH_BEFORE);

        assert_eq!(
            stale_paths(root.path()),
            Vec::<String>::new(),
            "case `{}`: fixture must start verify-clean, or the assertion below proves nothing",
            case.name
        );

        let files = vec![GeneratedFile {
            path: PathBuf::from("packages/go").join("go.mod"),
            content: SEED_BODY.to_owned(),
            generated_header: false,
        }];
        let before = std::fs::read_to_string(&path).expect("read seeded file");

        let paths = stampable_output_paths(&files, root.path());
        let updated = finalize_hashes(&paths, case.sources_hash_after, case.alef_toml_after).expect("finalize hashes");

        assert_eq!(
            updated, 0,
            "case `{}`: a file whose emitted content did not change must not be rewritten just \
             because generation inputs moved -- rewriting it is exactly the whole-tree \
             provenance churn this fix exists to stop",
            case.name
        );
        assert_eq!(
            stale_paths(root.path()),
            Vec::<String>::new(),
            "case `{}`: the file must still verify clean after the no-op finalize",
            case.name
        );

        let after = std::fs::read_to_string(&path).expect("read file after finalize");
        assert_eq!(
            after, before,
            "case `{}`: an unrelated generation-inputs change must not touch this file's bytes \
             at all, not even its hash line",
            case.name
        );
    }
}

/// The half that keeps the fix from degenerating into "stamp every path handed to me".
/// A generated file with neither an in-memory marker nor one on disk is not alef-stamped
/// output, and `alef verify` never walks it -- adding it to the stamp scope would mint a
/// marker-less file into the stamped set.
#[test]
fn unmarked_output_with_no_marker_on_disk_stays_out_of_the_stamp_scope() {
    let root = tempfile::tempdir().expect("tempdir");
    let relative = "packages/go/unmarked.mod";
    let path = join(root.path(), relative);
    std::fs::create_dir_all(path.parent().expect("fixture path has a parent")).expect("create parent dirs");
    std::fs::write(&path, SEED_BODY).expect("write unmarked file");

    let files = vec![GeneratedFile {
        path: PathBuf::from("packages/go").join("unmarked.mod"),
        content: SEED_BODY.to_owned(),
        generated_header: false,
    }];

    assert!(
        stampable_output_paths(&files, root.path()).is_empty(),
        "a file that carries no marker in memory and none on disk is not stamped output"
    );
}

/// A file alef marks in memory must be in scope whether or not it already exists, so the
/// disk-aware branch is an addition to the old predicate rather than a replacement for it.
#[test]
fn in_memory_marked_output_stays_in_the_stamp_scope_even_when_absent_from_disk() {
    let root = tempfile::tempdir().expect("tempdir");
    let files = vec![GeneratedFile {
        path: PathBuf::from("packages/go").join("binding.go"),
        content: "package fixture\n".to_owned(),
        generated_header: true,
    }];

    let paths = stampable_output_paths(&files, root.path());
    assert_eq!(
        paths,
        std::iter::once(join(root.path(), "packages/go/binding.go")).collect(),
        "the in-memory marker must still put a path in scope on its own"
    );
}
