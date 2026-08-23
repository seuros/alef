//! Guards for [`super::inline_workspace_lints`]: a crate lifted out of its
//! workspace must keep the `[workspace.lints]` it was inheriting.
//!
//! Split from `tests.rs` rather than appended to it — that file was already past
//! the 800-line "split before adding behaviour" mark, and lint inheritance is its
//! own concern with its own fixture workspace.

use super::*;
use std::fs;
use tempfile::TempDir;

/// A workspace whose `[workspace.lints]` carries the two shapes a vendored crate
/// needs in order to keep compiling outside its workspace: a bare level string,
/// and the `unexpected_cfgs` check-cfg allowlist that declares the crate's own
/// conditional-compilation gates. Without the allowlist, a `cfg(sample_gate)` in
/// the crate's sources is an `unexpected_cfgs` diagnostic — a warning nobody sees
/// in a default build, and a hard error under the `RUSTFLAGS="-D warnings"` that
/// CI sets. That asymmetry is why this guard exists at the manifest level: a
/// local `alef test` run cannot observe the regression at all.
fn setup_workspace_with_lints(root: &Path) {
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib"]

[workspace.package]
version = "1.2.3"
edition = "2024"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }

[workspace.lints.rust]
unused_must_use = "deny"
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(sample_gate)', 'cfg(feature, values("sample_kit"))'] }

[workspace.lints.clippy]
dbg_macro = "deny"
"#,
    )
    .unwrap();
    write_core_crate(root, "[lints]\nworkspace = true\n");
}

/// The same fixture minus any `[workspace.lints]`, for the nothing-to-inline path.
fn setup_workspace_without_lints(root: &Path) {
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib"]

[workspace.package]
version = "1.2.3"
edition = "2024"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
"#,
    )
    .unwrap();
    write_core_crate(root, "[lints]\nworkspace = true\n");
}

fn write_core_crate(root: &Path, lints_block: &str) {
    let core_dir = root.join("crates/my-lib/src");
    fs::create_dir_all(&core_dir).unwrap();
    fs::write(
        core_dir.join("lib.rs"),
        "#[cfg_attr(sample_gate, allow(dead_code))]\npub fn hello() {}\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/my-lib/Cargo.toml"),
        format!(
            r#"
[package]
name = "my-lib"
version.workspace = true
edition.workspace = true

[dependencies]
serde = {{ workspace = true }}

{lints_block}"#
        ),
    )
    .unwrap();
}

fn vendor_fixture(root: &Path) -> String {
    let dest = root.join("vendor");
    fs::create_dir_all(&dest).unwrap();
    let result = vendor_core_only(root, &root.join("crates/my-lib"), &dest, true).unwrap();
    fs::read_to_string(result.vendor_dir.join("Cargo.toml")).unwrap()
}

#[test]
fn vendored_manifest_keeps_workspace_check_cfg_declaration() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace_with_lints(root);

    let vendored = vendor_fixture(root);
    let doc: DocumentMut = vendored.parse().unwrap();

    assert!(
        vendored.contains("[lints.rust]"),
        "inherited lints must land as real manifest tables, not a stray empty header; got:\n{vendored}"
    );

    let lints = doc
        .get("lints")
        .and_then(|l| l.as_table_like())
        .unwrap_or_else(|| panic!("vendored manifest must declare [lints]; got:\n{vendored}"));

    assert!(
        lints.get("workspace").is_none(),
        "vendored manifest must not keep `workspace = true` — there is no parent workspace; got:\n{vendored}"
    );

    let rust = lints
        .get("rust")
        .and_then(|r| r.as_table_like())
        .unwrap_or_else(|| panic!("vendored manifest must declare [lints.rust]; got:\n{vendored}"));

    let rendered = rust
        .get("unexpected_cfgs")
        .unwrap_or_else(|| panic!("vendored [lints.rust] must keep `unexpected_cfgs`; got:\n{vendored}"))
        .to_string();
    assert!(
        rendered.contains("check-cfg"),
        "vendored `unexpected_cfgs` must keep its check-cfg allowlist; got: {rendered}"
    );
    assert!(
        rendered.contains("cfg(sample_gate)"),
        "vendored check-cfg must still declare every cfg name the source workspace declared; got: {rendered}"
    );
    assert!(
        rendered.contains(r#"cfg(feature, values("sample_kit"))"#),
        "vendored check-cfg must survive verbatim, feature-valued entries included; got: {rendered}"
    );

    assert_eq!(
        rust.get("unused_must_use")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("inherited `unused_must_use` must survive; got:\n{vendored}")),
        "deny",
        "every inherited [workspace.lints.rust] entry must survive, not just check-cfg"
    );

    let clippy = lints
        .get("clippy")
        .and_then(|c| c.as_table_like())
        .unwrap_or_else(|| panic!("vendored manifest must keep [lints.clippy] too; got:\n{vendored}"));
    assert_eq!(
        clippy.get("dbg_macro").and_then(|v| v.as_str()),
        Some("deny"),
        "inherited clippy lints must survive vendoring; got:\n{vendored}"
    );
}

#[test]
fn vendored_manifest_drops_lints_when_workspace_declares_none() {
    // The nothing-to-inline path: leaving `workspace = true` behind would make
    // the standalone manifest unreadable to cargo, so it still has to go.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace_without_lints(root);

    let vendored = vendor_fixture(root);
    let doc: DocumentMut = vendored.parse().unwrap();
    assert!(
        doc.get("lints").is_none(),
        "nothing to inline — the inheritance marker must go; got:\n{vendored}"
    );
}

#[test]
fn vendored_manifest_leaves_a_crate_owned_lints_table_alone() {
    // A crate that spells its own lints out (rather than inheriting) is not
    // inheriting anything, so vendoring must not overwrite its table with the
    // workspace's — binding crates do exactly this to opt out of an
    // all-or-nothing workspace policy they cannot satisfy.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace_with_lints(root);
    write_core_crate(root, "[lints.clippy]\nprint_stdout = \"deny\"\n");

    let vendored = vendor_fixture(root);
    let doc: DocumentMut = vendored.parse().unwrap();
    let lints = doc
        .get("lints")
        .and_then(|l| l.as_table_like())
        .unwrap_or_else(|| panic!("crate-owned [lints] must survive; got:\n{vendored}"));

    assert_eq!(
        lints
            .get("clippy")
            .and_then(|c| c.as_table_like())
            .and_then(|c| c.get("print_stdout"))
            .and_then(|v| v.as_str()),
        Some("deny"),
        "the crate's own lint table must be left verbatim; got:\n{vendored}"
    );
    assert!(
        lints.get("rust").is_none(),
        "workspace lints must not be spliced into a crate that opted out; got:\n{vendored}"
    );
}
