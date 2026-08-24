use super::{ScanCoverage, is_scannable, scan};

/// Seed one stamped file per name and return the names the ownership walk actually opened,
/// sorted. Mirrors `helpers::tests::scanned_names`; kept here so this module's own tests do
/// not depend on a sibling test module's private helper.
fn scanned_names(names: &[&str]) -> Vec<String> {
    let directory = tempfile::tempdir().expect("temporary project");
    for name in names {
        let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        std::fs::write(directory.path().join(name), format!("{marker}\nseeded = true\n")).expect("seed stamped file");
    }
    let mut found: Vec<String> = scan(directory.path())
        .0
        .into_iter()
        .filter_map(|(path, _, _)| path.file_name()?.to_str().map(str::to_owned))
        .collect();
    found.sort();
    found
}

/// Names the emit table (`cli::pipeline::generate::write::marker_header_syntax`) keys on by
/// FILE NAME rather than by extension. Every one of them is stamped on write, and every one
/// of them is invisible to `Path::extension`, so the read side can only reach them by name --
/// which is exactly where the two sides drifted.
const EMIT_SIDE_FILENAME_KEYED: &[&str] = &[
    "Makefile",
    "GNUmakefile",
    "go.mod",
    "Rakefile",
    "Makevars",
    "Makevars.in",
    "Makevars.win.in",
    ".clang-format",
];

/// THE Part 2 regression, in its one concrete measured instance.
///
/// `.clang-format` is scaffolded `generated_header: true` for every FFI target and is stamped
/// on write (it is YAML, so `#` line comments apply), but a dotfile with a single leading dot
/// reports `Path::extension() == None` and the name was never added to
/// `VERIFY_SCAN_FILENAMES`. The walk filters on name and extension BEFORE reading any content,
/// so the marker alef had just written was unreachable: the file was not merely unverified, it
/// was unverifiable, and a green `alef verify` said nothing about it either way.
///
/// Asserted through the real walk, not through `is_scannable` alone, so deleting the filter
/// clause without deleting the walk still fails here. ~keep
#[test]
fn walk_opens_a_stamped_clang_format() {
    assert_eq!(scanned_names(&[".clang-format"]), vec![".clang-format"]);
}

/// The general form of the same defect: anything the emit side stamps by file name must be
/// reachable by the read side. Driven from the emit predicate itself
/// (`is_markable_path`) rather than from a second hand-written list, so a new
/// name added to the emit table is covered here the day it lands. ~keep
#[test]
fn every_filename_keyed_stamped_path_is_scannable() {
    for name in EMIT_SIDE_FILENAME_KEYED {
        let path = std::path::Path::new(name);
        assert!(
            crate::cli::pipeline::is_markable_path(path),
            "{name} is listed as emit-side stamped but the emit table does not stamp it"
        );
        assert!(
            is_scannable(path),
            "{name} is stamped on write but the ownership walk would never open it"
        );
    }
}

/// The other half of the predicate. Without this, widening `is_scannable` to `true` would
/// satisfy every assertion above while turning the walk into "open every file in the tree",
/// which is both slow and a licence to read foreign content. ~keep
#[test]
fn walk_still_skips_a_format_alef_never_stamps() {
    assert!(scanned_names(&["notes.rtf", "archive.tar"]).is_empty());
    assert!(!is_scannable(std::path::Path::new("notes.rtf")));
}

/// The coverage tally must count what the walk did, not what it found.
///
/// A verdict computed from `marked` alone reads as a whole-tree claim; these three numbers are
/// what make the difference visible in the report. Seeded with one file of each kind so a
/// tally that silently collapsed two of the categories cannot pass. ~keep
#[test]
fn scan_tallies_opened_marked_and_unexamined_separately() {
    let directory = tempfile::tempdir().expect("temporary project");
    let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    std::fs::write(
        directory.path().join("stamped.toml"),
        format!("{marker}\nseeded = true\n"),
    )
    .expect("seed stamped file");
    std::fs::write(directory.path().join("plain.toml"), "seeded = true\n").expect("seed unmarked scannable file");
    std::fs::write(directory.path().join("notes.rtf"), "not scanned\n").expect("seed unscannable file");

    let (found, coverage) = scan(directory.path());
    assert_eq!(found.len(), 1, "only the stamped file is alef-owned");
    assert_eq!(
        coverage,
        ScanCoverage {
            opened: 2,
            marked: 1,
            unexamined: 1,
        }
    );
}
