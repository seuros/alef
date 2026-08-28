//! A scaffold file alef *claims* must never read as cached until it is *stamped*.
//!
//! Sibling to `marker_stamping`, one step further down the pipeline. That module proves the
//! emitters put a stamp where `poly` can see it. This one proves the `.alef/` stage cache does not
//! declare victory over a file that never got one.
//!
//! The failure this guards is not a wrong stamp, it is a *missing* one that becomes permanent.
//! `write_scaffold_files*` puts the prose header on disk and `finalize_hashes` adds the
//! `alef:hash:` line in a second pass, so between the two every marker-rail file is
//! claimed-but-unstamped. Any run that ends between them (an aborted stage, an interrupted
//! process) leaves that state committed. `stamped_outputs_agree_with_disk` used to read a missing
//! stamp as agreement, so the next run's `is_stage_cached` hit, skipped `finalize_hashes`, and the
//! file stayed unstamped: outside poly's hash-keyed skip, so `poly fmt` reformats it, and inside
//! alef's write set, so the next alef run writes its own bytes back. Neither tool yields.
//!
//! Repo-root scaffold files are the population with no way out of that on their own:
//! `.cargo/config.toml`, `rust-toolchain.toml`, `poly.toml` and `rustfmt.toml` sit outside every
//! root `generate::orphans::generate_sweep_roots` returns, so `finalize_hashes_sweeping`'s
//! disk-scan self-heal never scans them. For those the cache verdict is the only thing standing
//! between a one-run interruption and a permanently churning file.

use super::*;
use crate::cli::cache::stamped_outputs_agree_with_disk;
use crate::cli::pipeline::generate::ensure_generated_header;
use crate::core::hash::{compute_file_hash, content_has_alef_marker, inject_hash_line};

/// The bytes a marker-rail file holds on disk after the write pass and *before* `finalize_hashes`
/// runs -- the exact window an interrupted run freezes. ~keep
fn written_but_unstamped(file: &GeneratedFile) -> String {
    if file.generated_header {
        ensure_generated_header(&file.path, &file.content)
    } else {
        file.content.clone()
    }
}

/// One manifested output, written into a scratch tree, with the manifest the stage cache reads.
///
/// Returns `stamped_outputs_agree_with_disk`'s verdict for a single-entry manifest, which is what
/// `is_stage_cached` ANDs into its answer.
fn cache_agrees_for(body: &str, relative_path: &std::path::Path) -> bool {
    let root = tempfile::tempdir().expect("scratch tree");
    let output = root.path().join(relative_path);
    std::fs::create_dir_all(output.parent().expect("output path has a parent")).expect("output parent");
    std::fs::write(&output, body).expect("write manifested output");
    let manifest = root.path().join("stage.manifest");
    std::fs::write(&manifest, format!("{}\n", output.display())).expect("write manifest");
    stamped_outputs_agree_with_disk(&manifest)
}

/// Every marker-rail scaffold file this build emits, keyed by its relative path.
///
/// Built by sweeping `Language::ALL` through `scaffold` -- so a language or emitter added later is
/// covered without editing this file -- plus the three seeds `scaffold` suppresses whenever the
/// **process CWD** already carries them. alef's own repo root has a `rust-toolchain.toml` and a
/// `.cargo/config.toml`, so a `scaffold`-driven sweep alone would silently examine neither, which
/// is exactly how a regression on the two files this test exists for stays invisible. See
/// `marker_stamping`'s notes on the same CWD gate. ~keep
fn marker_rail_files() -> Vec<GeneratedFile> {
    let api = test_api();
    let config = test_config();
    let mut by_path: std::collections::BTreeMap<String, GeneratedFile> = std::collections::BTreeMap::new();

    for language in Language::ALL {
        let Ok(files) = scaffold(&api, &config, &[language]) else {
            continue;
        };
        for file in files {
            if !content_has_alef_marker(&written_but_unstamped(&file)) {
                continue;
            }
            by_path.insert(file.path.to_string_lossy().into_owned(), file);
        }
    }

    for seed in [
        rust_toolchain_file(&[Language::Wasm]),
        wasm_cargo_config_file(),
        GeneratedFile {
            path: std::path::PathBuf::from(".cargo/config.toml"),
            content: render_cargo_config(&crate::core::config::ScaffoldCargo::default()),
            generated_header: true,
        },
    ] {
        assert!(
            content_has_alef_marker(&written_but_unstamped(&seed)),
            "{}: CWD-gated seed no longer carries an alef marker, so the sweep below would drop it \
             and this test would stop covering the file it exists for",
            seed.path.display()
        );
        by_path.insert(seed.path.to_string_lossy().into_owned(), seed);
    }

    by_path.into_values().collect()
}

/// Floor for the emitter table. Set well under the observed population so ordinary churn does not
/// trip it, and well above zero so a sweep that stops finding emitters fails instead of reporting
/// success over nothing. ~keep
const MINIMUM_MARKER_RAIL_FILES: usize = 10;

/// The repo-root files with no `generate_sweep_roots` fallback. Named individually because they
/// are the reason this test exists: for every other marker-rail file the disk-scan self-heal is a
/// second chance, and for these four the cache verdict is the only one. ~keep
const ROOT_SCAFFOLD_FILES: &[&str] = &[".cargo/config.toml", "rust-toolchain.toml", "poly.toml", "rustfmt.toml"];

#[test]
fn a_claimed_but_unstamped_scaffold_output_is_never_read_as_cached() {
    let files = marker_rail_files();

    assert!(
        files.len() >= MINIMUM_MARKER_RAIL_FILES,
        "the emitter table holds only {} marker-rail scaffold file(s), below the \
         {MINIMUM_MARKER_RAIL_FILES} floor -- it has gone vacuous and would pass no matter what the \
         cache does. Found: {:?}",
        files.len(),
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
    );
    let found: std::collections::BTreeSet<String> =
        files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
    for expected in ROOT_SCAFFOLD_FILES {
        assert!(
            found.contains(*expected),
            "{expected} is a repo-root marker-rail scaffold file with no sweep-root self-heal, but \
             the emitter table never produced it, so this test does not cover it. Found: {found:?}"
        );
    }

    for file in &files {
        let unstamped = written_but_unstamped(file);
        assert!(
            crate::core::hash::extract_hash(&unstamped).is_none(),
            "{}: the pre-`finalize_hashes` bytes already carry an `alef:hash:` line, so this row \
             is not the unstamped state it is supposed to stand for and asserts nothing",
            file.path.display()
        );
        assert!(
            !cache_agrees_for(&unstamped, &file.path),
            "{}: alef claims this file (its prose marker is on disk) but has not stamped it, and \
             the stage cache still reports agreement. That verdict skips the `finalize_hashes` \
             pass that would add the `alef:hash:` line, so the file stays outside poly's \
             hash-keyed skip: `poly fmt` reformats it, the next alef run writes its own bytes \
             back, and neither tool yields. Header written was:\n{}",
            file.path.display(),
            unstamped.lines().take(3).collect::<Vec<_>>().join("\n")
        );
    }
}

/// Control: the predicate must still say `true` for a properly stamped file, or the assertion
/// above would pass on a function that returns `false` unconditionally.
#[test]
fn a_stamped_scaffold_output_still_reads_as_cached() {
    let files = marker_rail_files();
    assert!(
        !files.is_empty(),
        "the emitter table is empty, so this control examines nothing"
    );

    for file in &files {
        let unstamped = written_but_unstamped(file);
        let stamped = inject_hash_line(&unstamped, &compute_file_hash(&unstamped));
        assert!(
            stamped.contains("alef:hash:"),
            "{}: control setup failed -- `inject_hash_line` produced no stamp, so the assertion \
             below would be checking an unstamped file and could not distinguish the two cases",
            file.path.display()
        );
        assert!(
            cache_agrees_for(&stamped, &file.path),
            "{}: a freshly stamped output must read as cached; if it does not, every stage would \
             regenerate on every run",
            file.path.display()
        );
    }
}

/// Control: an output with no alef marker at all -- a `generated_header: false` create-once seed,
/// or a format with no comment syntax -- keeps the existence-only rule. Widening the miss to every
/// unstamped file would make those stages permanently cold, since alef never stamps them.
#[test]
fn an_unmarked_create_once_seed_still_reads_as_cached() {
    for (relative_path, body) in [
        ("packages/php/composer.json", "{\n  \"name\": \"vendor/sample\"\n}\n"),
        ("packages/zig/build.zig", "pub fn build() void {}\n"),
    ] {
        let path = std::path::Path::new(relative_path);
        assert!(
            !content_has_alef_marker(body),
            "{relative_path}: control fixture accidentally carries an alef marker, so it no longer \
             stands for the unmarked-seed case"
        );
        assert!(
            cache_agrees_for(body, path),
            "{relative_path}: an unmarked create-once seed has no stamp to compare and must keep \
             the existence-only rule, or its stage never hits the cache again"
        );
    }
}
