//! Regression test for `alef snippets check`'s documented contract.
//!
//! `alef snippets check --help` promises the subcommand runs "the configured
//! snippet discovery, validation, audit, and gap checks", but `run_check`
//! used to only run discovery and validation. A crate whose docs reference a
//! snippet that does not exist on disk — a structural gap that
//! `alef snippets audit` and `alef snippets gaps` both already catch on their
//! own — passed `check` silently. This test builds a minimal self-contained
//! workspace with exactly that dangling reference and drives the real
//! compiled binary (`std::process::ExitCode` has no public accessor to
//! compare a library-level `run()` result against, so this exercises the CLI
//! end to end instead) to assert `check` now fails on it.

use std::path::Path;
use std::process::Command;

/// Writes a minimal `alef.toml` workspace fixture with one JSON snippet
/// (validates without any external toolchain) and one docs file that
/// `--8<--`-includes a snippet path that does not exist.
fn write_fixture_with_dangling_include(root: &Path) {
    std::fs::create_dir_all(root.join("snippets")).expect("create snippets dir");
    std::fs::create_dir_all(root.join("docs")).expect("create docs dir");

    std::fs::write(
        root.join("snippets/hello.md"),
        "# Hello\n\n```json\n{\"hello\": \"world\"}\n```\n",
    )
    .expect("write snippet fixture");

    std::fs::write(root.join("docs/guide.md"), "# Guide\n\n--8<-- \"missing-fixture.md\"\n")
        .expect("write docs fixture");

    std::fs::write(
        root.join("alef.toml"),
        r#"
[workspace]
languages = ["python"]

[workspace.docs.snippets]
dirs = ["snippets"]
docs_dirs = ["docs"]

[[crates]]
name = "fixture-crate"
sources = ["src/lib.rs"]
"#,
    )
    .expect("write alef.toml");
}

/// Writes a minimal `alef.toml` workspace fixture whose `docs.snippets.inline_dirs` points at a
/// path that was never created, alongside one real, passing snippet in `dirs`.
///
/// `inline_dirs` is deliberately used here rather than `dirs`: `run_check`'s audit/gap pass
/// (`run_configured_audit_and_gaps`) only ever sees `docs.snippets.dirs`, so a missing `dirs`
/// entry happens to fail today through an unrelated coverage-ledger walk. `inline_dirs` is not
/// passed to that pass at all, so a missing `inline_dirs` root previously reached only
/// `discover_snippets`, which silently skipped it -- this is the exact shape of the bug: a
/// snippets root that was repointed but never populated read as a clean run. ~keep
fn write_fixture_with_missing_inline_dir(root: &Path) {
    std::fs::create_dir_all(root.join("snippets")).expect("create snippets dir");

    std::fs::write(root.join("snippets/hello.md"), "```json\n{\"hello\": \"world\"}\n```\n")
        .expect("write snippet fixture");

    std::fs::write(
        root.join("alef.toml"),
        r#"
[workspace]
languages = ["python"]

[workspace.docs.snippets]
dirs = ["snippets"]
inline_dirs = ["generated-snippets-not-yet-produced"]

[[crates]]
name = "fixture-crate"
sources = ["src/lib.rs"]
"#,
    )
    .expect("write alef.toml");
}

#[test]
fn check_fails_when_an_inline_dirs_root_does_not_exist() {
    let dir = tempfile::tempdir().expect("create temp workspace");
    write_fixture_with_missing_inline_dir(dir.path());

    let output = Command::new(env!("CARGO_BIN_EXE_alef"))
        .args(["snippets", "check", "--config"])
        .arg(dir.path().join("alef.toml"))
        .args(["--cache", "off"])
        .env("RUST_LOG", "info")
        .output()
        .expect("run the alef binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let context = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        !output.status.success(),
        "`check` must fail when a configured snippets root does not exist on disk — a repointed- \
         but-never-populated directory must never read as an exhaustively validated one.\n{context}"
    );
    assert!(
        stderr.contains("generated-snippets-not-yet-produced"),
        "the diagnostic must name the missing directory so the misconfiguration is actionable, not \
         just report a generic discovery failure.\n{context}"
    );
}

#[test]
fn check_fails_on_a_dangling_include_target() {
    let dir = tempfile::tempdir().expect("create temp workspace");
    write_fixture_with_dangling_include(dir.path());

    let output = Command::new(env!("CARGO_BIN_EXE_alef"))
        .args(["snippets", "check", "--config"])
        .arg(dir.path().join("alef.toml"))
        .args(["--cache", "off"])
        .env("RUST_LOG", "info")
        .output()
        .expect("run the alef binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let context = format!("stdout:\n{stdout}\nstderr:\n{stderr}");

    assert!(
        !output.status.success(),
        "`check` must fail when a docs file includes a snippet that does not exist — \
         this is exactly what `alef snippets audit` and `alef snippets gaps` already \
         detect, and `check`'s own --help text promises it runs both.\n{context}"
    );
    assert!(
        stderr.contains("snippet audit:") && stderr.contains("included snippet does not exist"),
        "`check` must fail *because of* the audit finding. `run_check` returns FAILURE from ~8 other \
         sites (config load, discovery, session resolution, report writing, ...), so asserting on the \
         exit status alone would keep this test green while testing nothing.\n{context}"
    );
    assert!(
        stderr.contains("snippet gap: missing include target"),
        "the gap pass must report the same dangling include; the binary installs a stderr `fmt` \
         subscriber at `info` by default, so its ERROR events are observable here.\n{context}"
    );
    assert!(
        stderr.contains("missing-fixture.md"),
        "the diagnostic must name the include target that could not be resolved.\n{context}"
    );
}
