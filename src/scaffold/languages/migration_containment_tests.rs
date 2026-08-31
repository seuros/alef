//! The write boundary around the in-place scaffold *migrations*.
//!
//! `symlink_containment_tests` covers the two report writers. Those are not the only writers:
//! every migration below repairs a `generated_header: false` create-once file through its own
//! `NamedTempFile::new_in(path.parent())` plus `persist`, so each is an independent write sink,
//! and two of them (`.cargo/config.toml`, `poly.toml`) run unconditionally on paths that never
//! enter the emitted file list at all -- the report writers' guard cannot have seen them.
//!
//! The paths split two ways, and both are covered here because the *lexical* origin of a
//! component is not what makes a sink safe:
//!
//! - project-derived, where a component comes from `alef.toml`: the swift module directory
//!   (`packages/swift/Tests/<Module>Tests`, from `ResolvedCrateConfig::swift_module`) and the
//!   dart test file (`packages/dart/test/<pkg>_test.dart`).
//! - fixed literal, where every component is a compile-time string: `.cargo/config.toml`,
//!   `poly.toml`, `packages/kotlin/build.gradle.kts`.
//!
//! A fixed literal is not thereby safe. `create_dir_all`, `NamedTempFile::new_in` and `persist`
//! all resolve symlinks, so `.cargo` being a symlink carries a wholly-literal path out of the
//! project just as effectively as a hostile module name does. Both classes therefore route
//! through the same `contained_output_path` gate, and both are asserted here.
//!
//! Unix-only, for the reason `symlink_containment_tests` is: staging the escape needs
//! `std::os::unix::fs::symlink`. The production check is not gated.
//!
//! The green controls are load-bearing in the other direction: a guard that rejected every
//! pre-existing ancestor, or every base reached through a symlink, would satisfy every refusal
//! test above and break every real repair on macOS, where `/tmp` and `/var/folders` (the home of
//! `tempfile::tempdir`) are themselves symlinks. ~keep
#![cfg(unix)]

use super::{
    STALE_WASM_CARGO_CONFIG, migrate_dart_placeholder_test, migrate_kotlin_build_gradle,
    migrate_poly_toml_drop_snippet_hook, migrate_swift_placeholder_test,
    migrate_wasm_cargo_config_allow_multiple_definition,
};
use std::path::Path;

const SWIFT_RELATIVE: &str = "packages/swift/Tests/EvilTests/EvilTests.swift";
const SWIFT_PLACEHOLDER: &str = "import XCTest\n\nfinal class EvilTests: XCTestCase {\n    func testPlaceholder() {\n        XCTAssertTrue(true)\n    }\n}\n";
const SWIFT_REPLACEMENT: &str = "import XCTest\n\nfinal class EvilTests: XCTestCase {\n    func testRoundTrip() {\n        XCTAssertEqual(greet(), \"hi\")\n    }\n}\n";

const DART_RELATIVE: &str = "packages/dart/test/evil_test.dart";
const DART_PLACEHOLDER: &str = "void main() {\n  test('placeholder', () {\n    expect(1 + 1, equals(2));\n  });\n}\n";
const DART_REPLACEMENT: &str = "void main() {\n  it('greets', () {\n    check(greet(), equals('hi'));\n  });\n}\n";

const KOTLIN_RELATIVE: &str = "packages/kotlin/build.gradle.kts";
const STALE_BUILD_GRADLE: &str =
    "mavenPublishing {\n  configure(\n    KotlinJvm(\n      sourcesJar = true,\n    )\n  )\n}\n";

const POLY_RELATIVE: &str = "poly.toml";
const STALE_POLY_TOML: &str =
    "[hooks.pre-commit.commands.alef-snippets]\nrun = \"alef snippets check --strict --cache off\"\nworkspace = true\n";

const CARGO_RELATIVE: &str = ".cargo/config.toml";

fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("symlink");
}

fn seed(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().expect("seeded path has a parent")).expect("create parent directory");
    std::fs::write(path, content).expect("seed file");
}

/// Assert `outside` still holds exactly the file it was staged with, byte for byte.
///
/// Both halves matter and neither implies the other. Unchanged *content* proves the repair did
/// not land out here; an unchanged *entry count* proves the refusal fired ahead of
/// `NamedTempFile::new_in`, which would otherwise leave an abandoned `.tmpXXXXXX` behind even on
/// a run that never reached `persist`. ~keep
fn assert_untouched(outside: &Path, name: &str, content: &str) {
    let entries: Vec<_> = std::fs::read_dir(outside)
        .expect("read outside directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "migration left residue outside the project: {entries:?}"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join(name)).expect("outside file"),
        content,
        "migration rewrote a file outside the project"
    );
}

#[test]
fn swift_placeholder_migration_refuses_a_symlinked_ancestor_that_leaves_the_project() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(base.join("packages/swift/Tests")).expect("Tests directory");
    seed(&outside.join("EvilTests.swift"), SWIFT_PLACEHOLDER);
    symlink(&outside, &base.join("packages/swift/Tests/EvilTests"));

    let error = migrate_swift_placeholder_test(&base, Path::new(SWIFT_RELATIVE), SWIFT_REPLACEMENT)
        .expect_err("a symlinked module directory must be refused");

    assert!(error.to_string().contains("not contained"), "{error}");
    assert_untouched(&outside, "EvilTests.swift", SWIFT_PLACEHOLDER);
}

#[test]
fn dart_placeholder_migration_refuses_a_symlinked_ancestor_that_leaves_the_project() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(base.join("packages/dart")).expect("dart directory");
    seed(&outside.join("evil_test.dart"), DART_PLACEHOLDER);
    symlink(&outside, &base.join("packages/dart/test"));

    let error = migrate_dart_placeholder_test(&base, Path::new(DART_RELATIVE), DART_REPLACEMENT)
        .expect_err("a symlinked test directory must be refused");

    assert!(error.to_string().contains("not contained"), "{error}");
    assert_untouched(&outside, "evil_test.dart", DART_PLACEHOLDER);
}

#[test]
fn cargo_config_migration_refuses_a_symlinked_dot_cargo_directory() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(&base).expect("base directory");
    seed(&outside.join("config.toml"), STALE_WASM_CARGO_CONFIG);
    symlink(&outside, &base.join(".cargo"));

    let error = migrate_wasm_cargo_config_allow_multiple_definition(&base)
        .expect_err("a symlinked .cargo directory must be refused");

    assert!(error.to_string().contains("not contained"), "{error}");
    assert_untouched(&outside, "config.toml", STALE_WASM_CARGO_CONFIG);
}

#[test]
fn poly_toml_migration_refuses_a_symlinked_leaf_pointing_outside_the_project() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(&base).expect("base directory");
    seed(&outside.join(POLY_RELATIVE), STALE_POLY_TOML);
    symlink(&outside.join(POLY_RELATIVE), &base.join(POLY_RELATIVE));

    let error = migrate_poly_toml_drop_snippet_hook(&base).expect_err("a poly.toml symlinked outside must be refused");

    assert!(error.to_string().contains("not contained"), "{error}");
    assert_untouched(&outside, POLY_RELATIVE, STALE_POLY_TOML);
}

#[test]
fn kotlin_build_gradle_migration_refuses_a_symlinked_ancestor_that_leaves_the_project() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(base.join("packages")).expect("packages directory");
    seed(&outside.join("build.gradle.kts"), STALE_BUILD_GRADLE);
    symlink(&outside, &base.join("packages/kotlin"));

    let error = migrate_kotlin_build_gradle(&base).expect_err("a symlinked packages/kotlin directory must be refused");

    assert!(error.to_string().contains("not contained"), "{error}");
    assert_untouched(&outside, "build.gradle.kts", STALE_BUILD_GRADLE);
}

#[test]
fn swift_placeholder_migration_still_repairs_an_ordinary_directory() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    seed(&base.join(SWIFT_RELATIVE), SWIFT_PLACEHOLDER);

    let changed = migrate_swift_placeholder_test(&base, Path::new(SWIFT_RELATIVE), SWIFT_REPLACEMENT)
        .expect("an ordinary directory must still be repaired");

    assert!(changed, "the vacuous placeholder must be reported as repaired");
    assert_eq!(
        std::fs::read_to_string(base.join(SWIFT_RELATIVE)).expect("repaired file"),
        SWIFT_REPLACEMENT
    );
}

#[test]
fn dart_placeholder_migration_still_repairs_an_ordinary_directory() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    seed(&base.join(DART_RELATIVE), DART_PLACEHOLDER);

    let changed = migrate_dart_placeholder_test(&base, Path::new(DART_RELATIVE), DART_REPLACEMENT)
        .expect("an ordinary directory must still be repaired");

    assert!(changed, "the vacuous placeholder must be reported as repaired");
    assert_eq!(
        std::fs::read_to_string(base.join(DART_RELATIVE)).expect("repaired file"),
        DART_REPLACEMENT
    );
}

#[test]
fn cargo_config_migration_still_repairs_an_ordinary_directory() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    seed(&base.join(CARGO_RELATIVE), STALE_WASM_CARGO_CONFIG);

    let changed = migrate_wasm_cargo_config_allow_multiple_definition(&base)
        .expect("an ordinary .cargo directory must still be repaired");

    assert!(changed, "the stale wasm32 rustflags must be reported as repaired");
    let repaired = std::fs::read_to_string(base.join(CARGO_RELATIVE)).expect("repaired file");
    assert!(
        repaired.contains("--allow-multiple-definition"),
        "the repair did not reach the file: {repaired}"
    );
}

#[test]
fn poly_toml_migration_still_repairs_an_ordinary_repo_root() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    seed(&base.join(POLY_RELATIVE), STALE_POLY_TOML);

    let changed = migrate_poly_toml_drop_snippet_hook(&base).expect("an ordinary repo root must still be repaired");

    assert!(changed, "the retracted hook must be reported as removed");
    let repaired = std::fs::read_to_string(base.join(POLY_RELATIVE)).expect("repaired file");
    assert!(!repaired.contains("alef-snippets"), "the hook survived: {repaired}");
}

#[test]
fn kotlin_build_gradle_migration_still_repairs_an_ordinary_directory() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    seed(&base.join(KOTLIN_RELATIVE), STALE_BUILD_GRADLE);

    let changed = migrate_kotlin_build_gradle(&base).expect("an ordinary directory must still be repaired");

    assert!(changed, "the missing trailing comma must be reported as repaired");
    let repaired = std::fs::read_to_string(base.join(KOTLIN_RELATIVE)).expect("repaired file");
    assert!(
        repaired.contains("    ),\n  )\n"),
        "the trailing comma was not added: {repaired}"
    );
}

#[test]
fn symlinked_ancestor_that_stays_inside_the_project_is_still_repaired() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let real = base.join("real-tests");
    std::fs::create_dir_all(&real).expect("real tests directory");
    std::fs::create_dir_all(base.join("packages/swift/Tests")).expect("Tests directory");
    symlink(&real, &base.join("packages/swift/Tests/EvilTests"));
    std::fs::write(real.join("EvilTests.swift"), SWIFT_PLACEHOLDER).expect("seed file");

    let changed = migrate_swift_placeholder_test(&base, Path::new(SWIFT_RELATIVE), SWIFT_REPLACEMENT)
        .expect("a symlink that stays inside the project is not an escape");

    assert!(changed, "the vacuous placeholder must be reported as repaired");
    assert_eq!(
        std::fs::read_to_string(real.join("EvilTests.swift")).expect("repaired file"),
        SWIFT_REPLACEMENT
    );
}

#[test]
fn base_directory_reached_through_a_symlink_is_still_repaired() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let real_root = temporary.path().join("real-root");
    seed(&real_root.join("base").join(SWIFT_RELATIVE), SWIFT_PLACEHOLDER);
    let linked_root = temporary.path().join("linked-root");
    symlink(&real_root, &linked_root);

    // The whole `base_dir` is addressed through a symlink, exactly as it is for every consumer
    // whose checkout sits under a symlinked home, volume or `/tmp` -- macOS resolves `/tmp` to
    // `/private/tmp` and `/var/folders` to `/private/var/folders`. Containment must be judged
    // between canonical paths on both sides, or this legitimate repair is refused. ~keep
    let base = linked_root.join("base");
    let changed = migrate_swift_placeholder_test(&base, Path::new(SWIFT_RELATIVE), SWIFT_REPLACEMENT)
        .expect("a symlinked base_dir must not be treated as an escape");

    assert!(changed, "the vacuous placeholder must be reported as repaired");
    assert_eq!(
        std::fs::read_to_string(real_root.join("base").join(SWIFT_RELATIVE)).expect("repaired file"),
        SWIFT_REPLACEMENT
    );
}
