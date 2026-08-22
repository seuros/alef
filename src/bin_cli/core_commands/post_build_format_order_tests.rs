//! Regression coverage for the `alef generate` post-build/format ordering defect: a post-build
//! step that writes straight to disk, bypassing `write_files_report` (Swift's
//! `MaterializeSwiftBridge`, which materializes `RustBridgeC.h` from swift-bridge's own build
//! output), used to run AFTER the only formatting pass `Commands::Generate` performed. The file
//! this run shipped was therefore stamped (`finalize_hashes`) over whatever raw bytes the
//! post-build tool produced, never run through `poly fmt` at all.
//!
//! That is not merely cosmetic. `poly`'s built-in "hash-stamped generated file" skip is
//! content-pattern-based (see `core::hash::POLY_GENERATED_SCAN_LINES`): once a file carries a
//! well-formed `alef:hash:<hex>` line in its leading lines, poly leaves the file untouched on
//! every later invocation, canonical or not -- verified directly against the `poly` binary this
//! suite runs against, both in isolation and through this exact fixture. So a file stamped BEFORE
//! ever being formatted ships non-canonical forever: poly's own protection is what prevents any
//! later pass -- another `alef all` run, or a standalone `poly fmt --fix .` a consumer runs
//! before committing -- from ever bringing it into line, since poly cannot tell "stamped and
//! canonical" apart from "stamped and never formatted".
//!
//! `alef all` (`all_commands.rs`) already runs post-build before its whole-tree format pass and
//! does not have this defect; only `alef generate` (`Commands::Generate` in `core_commands.rs`)
//! did. This module exercises `Commands::Generate` directly (not `alef all`) so a regression in
//! the specific arm that was fixed is what fails, not a passing sibling command.

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::test_support::SkipCommandsGuard;
use std::path::Path;

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"
"#;

/// The fixed binding crate name `MaterializeSwiftBridge` derives from the fixture's crate name
/// (`format!("{}-swift", config.name)`), pinned here as a constant instead of recomputed inline
/// so the fake `target/` layout below and the assertions after `alef generate` runs can never
/// drift against each other.
const BINDING_CRATE_NAME: &str = "test-lib-swift";

/// Non-canonical (2-space) struct-field indentation, deliberately NOT matching poly's own C
/// style. Measured directly against the `poly` binary this suite runs against: an otherwise
/// identical file with no `alef:hash:` line gets exactly these two `typedef struct` blocks
/// reindented to 4 spaces by `poly fmt --fix`. Using content poly would genuinely rewrite -- not
/// content that happens to already be canonical -- is what makes the fixed-point assertions below
/// load-bearing rather than vacuously true either way.
const NON_CANONICAL_CORE_H: &str = "typedef struct RustStr {\n  uint8_t *const start;\n  uintptr_t len;\n} RustStr;\n";
const NON_CANONICAL_CRATE_H: &str = "typedef struct DemoOpaque {\n  void *const ptr;\n} DemoOpaque;\n";

fn write_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
    // `find_swift_bridge_out_dir` (`backends::swift::gen_bindings::bridge_artifacts`) walks
    // ancestors of the process cwd looking for a `Cargo.lock` to anchor its `target/` search --
    // content is never read, only existence, so an empty stub is sufficient.
    std::fs::write(root.join("Cargo.lock"), "").expect("write fixture Cargo.lock");
    // Works around a SEPARATE, pre-existing defect this fixture is not testing: both
    // `emit_swift_bridge_files` call sites (`gen_bindings/mod.rs`'s in-process call and
    // `MaterializeSwiftBridge`'s post-build call) resolve `package_root` via
    // `PathBuf::from(base_dir).ancestors().find(|p| p.join("Sources").is_dir())`, falling back to
    // `.parent().and_then(|p| p.parent())` when nothing matches. For `base_dir =
    // "packages/swift"` that fallback is `Some("packages").and_then(|p| p.parent())` --
    // `Path::new("packages").parent()` is `Some("")`, NOT `None` -- so the chain resolves to
    // `PathBuf::from("")` (the repo root) on a project with no `packages/swift/Sources` yet,
    // never reaching the final `unwrap_or_else(|| PathBuf::from("packages/swift"))`. On a
    // completely fresh project (this fixture, before this workaround) that misplaces the whole
    // `Sources/` tree at the repo root instead of under `packages/swift/`. Pre-creating an empty
    // `packages/swift/Sources` directory makes the primary `ancestors().find` branch match on its
    // very first candidate, sidestepping the buggy fallback entirely -- this is a workaround for
    // this fixture, not a fix; the underlying defect is out of scope for the post-build/format
    // ordering bug this module tests and is unrelated to it. ~keep
    std::fs::create_dir_all(root.join("packages/swift/Sources")).expect("create packages/swift/Sources");
}

/// Pre-seed `target/release/build/<binding_crate_name>-<hash>/out/` with the swift-bridge build
/// output `MaterializeSwiftBridge` would otherwise only get from a real `cargo build` invoking
/// the `swift-bridge-build` crate's build script -- a genuine build needs network-fetched
/// dependencies and a full swift-bridge toolchain neither available nor desirable in a unit test.
/// `find_swift_bridge_out_dir` only checks for a directory name prefix and a
/// `SwiftBridgeCore.swift` marker file inside `out/`, so a hand-built directory with the right
/// shape is indistinguishable from a real one to the code under test.
fn seed_fake_swift_bridge_build_output(root: &Path) {
    let out_dir = root
        .join("target/release/build")
        .join(format!("{BINDING_CRATE_NAME}-fakehash/out"));
    let binding_out_dir = out_dir.join(BINDING_CRATE_NAME);
    std::fs::create_dir_all(&binding_out_dir).expect("create fake swift-bridge out dir");
    std::fs::write(out_dir.join("SwiftBridgeCore.swift"), "import Foundation\n")
        .expect("write fake SwiftBridgeCore.swift");
    std::fs::write(out_dir.join("SwiftBridgeCore.h"), NON_CANONICAL_CORE_H).expect("write fake SwiftBridgeCore.h");
    std::fs::write(
        binding_out_dir.join(format!("{BINDING_CRATE_NAME}.swift")),
        "import Foundation\n",
    )
    .expect("write fake crate swift file");
    std::fs::write(
        binding_out_dir.join(format!("{BINDING_CRATE_NAME}.h")),
        NON_CANONICAL_CRATE_H,
    )
    .expect("write fake crate h file");
}

fn header_path(root: &Path) -> std::path::PathBuf {
    root.join("packages/swift/Sources/RustBridgeC/RustBridgeC.h")
}

/// Run `alef generate --lang swift` against `root`. `ALEF_SKIP_COMMANDS=cargo` skips the
/// `RunCommand` step that would otherwise `cargo build` a real swift-bridge crate before
/// `MaterializeSwiftBridge` runs -- `run_run_command` treats a listed command as a deterministic,
/// non-fatal `Ok(false)` skip (see that function's doc), not an error, so `MaterializeSwiftBridge`
/// still runs immediately after and finds the fake `out/` directory `seed_fake_swift_bridge_build_output`
/// planted. Guarded by `SkipCommandsGuard` because `ALEF_SKIP_COMMANDS` is process-global. ~keep
fn run_generate(root: &Path) {
    let _skip_guard = SkipCommandsGuard::set("cargo");
    let _cwd = crate::test_support::CwdGuard::enter(root);
    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Generate {
            lang: Some(vec!["swift".to_owned()]),
            clean: false,
            skip_frb: false,
            strict: false,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");
}

/// The regression test itself: `alef generate` must leave the post-build-owned header both
/// poly-canonical AND correctly stamped in the SAME pass -- no separate `poly fmt --fix .` step
/// in between. Before the fix (post-build running after the only format pass), this failed on
/// the `poly_fmt_check_reports_the_file_clean` assertion: the header shipped with the raw
/// 2-space `NON_CANONICAL_*` indentation, `poly fmt --check` reported it as needing a rewrite
/// (not skipped-as-stamped, since a freshly-generated file with a fresh but never-formatted body
/// still gets a fresh, well-formed hash line from `finalize_hashes` -- the bug is that the body
/// under that hash was never canonical to begin with). After the fix, post-build runs before the
/// format pass, so the body the hash covers IS the canonical one, and `poly fmt --check` finds
/// nothing to do. ~keep
#[test]
fn generate_formats_post_build_output_before_stamping_it() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    seed_fake_swift_bridge_build_output(&root);

    run_generate(&root);

    let header = header_path(&root);
    let content = std::fs::read_to_string(&header).expect("post-build must have materialized RustBridgeC.h");
    assert!(
        content.contains("alef:hash:"),
        "sanity: the header must actually be alef-stamped, or every assertion below is vacuous: {content}"
    );
    assert!(
        content.contains("typedef struct RustStr {\n    uint8_t"),
        "the header's body must be poly-canonical (4-space struct fields) by the time it ships -- \
         got:\n{content}"
    );

    // The literal regression assertion: `poly fmt --check` must find nothing to do. Before the
    // fix this failed here -- the shipped header still carried the RAW 2-space body under a
    // hash that matched it (self-consistent, but never canonical), and poly does not
    // distinguish "correctly stamped" from "stamped over non-canonical bytes".
    let check = std::process::Command::new("poly")
        .args(["fmt", "--check", "."])
        .current_dir(&root)
        .output()
        .expect("run poly fmt --check");
    let check_output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        check.status.success(),
        "`poly fmt --check .` must report the tree clean immediately after `alef generate` -- a \
         post-build-owned file was formatted after being stamped, not before. Output:\n{check_output}"
    );
}

/// The fixed-point property required alongside the regression test above: running `alef
/// generate` a second time, with nothing about the sources or fixture changed, must leave the
/// post-build-owned header byte-for-byte unchanged. Before the fix this still held for a
/// SINGLE run (the file was self-consistently stamped even though never formatted), so this test
/// alone would not have caught the defect -- it exists to prove the fix does not merely paper
/// over the first run while leaving a later reformat (a whole-tree `alef all`, or a standalone
/// `poly fmt --fix .`) able to invalidate the stamp again. ~keep
#[test]
fn generate_reaches_a_fixed_point_on_post_build_output_across_repeat_runs() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    seed_fake_swift_bridge_build_output(&root);

    run_generate(&root);
    let header = header_path(&root);
    let first_pass = std::fs::read_to_string(&header).expect("first alef generate must materialize the header");

    run_generate(&root);
    let second_pass = std::fs::read_to_string(&header).expect("second alef generate must leave the header in place");

    assert_eq!(
        first_pass, second_pass,
        "a second `alef generate` run over an unchanged tree must leave the post-build-owned \
         header byte-for-byte unchanged -- any diff here means the pipeline has not reached a \
         fixed point"
    );

    // Run once more through poly directly, outside alef entirely -- the exact "standalone `poly
    // fmt --fix .` before committing" trigger the observed incident described. A file that
    // reached a genuine fixed point on the first `alef generate` run must survive this
    // untouched too.
    let fix = std::process::Command::new("poly")
        .args(["fmt", "--fix", "."])
        .current_dir(&root)
        .output()
        .expect("run poly fmt --fix");
    assert!(
        fix.status.success(),
        "poly fmt --fix must exit 0 on an already-canonical tree"
    );
    let after_standalone_fmt = std::fs::read_to_string(&header).expect("header must still exist");
    assert_eq!(
        second_pass, after_standalone_fmt,
        "a standalone `poly fmt --fix .` run after `alef generate` must not change a byte of an \
         already-canonical, correctly-stamped post-build-owned file"
    );
}
